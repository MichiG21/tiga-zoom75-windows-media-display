//! The upload stream: validating it and cutting it into BLE writes.

use crate::protocol::*;
use std::error::Error;

#[derive(Clone)]
pub(crate) struct Chunk {
    pub(crate) value: Vec<u8>,
    pub(crate) ends_4k_block: bool,
}

pub(crate) fn xor8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |acc, b| acc ^ b)
}

pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

pub(crate) fn validate_stream(stream: &[u8], allow_single_frame: bool) -> Result<(), Box<dyn Error>> {
    let single = SINGLE_FRAME_STREAM_LENS.contains(&stream.len());
    if single && !allow_single_frame {
        return Err(format!(
            "stream size {} is an experimental single-frame container; pass --allow-single-frame",
            stream.len()
        ).into());
    }
    if stream.len() != STREAM_LEN && !single {
        return Err(format!("stream size {} != expected {}", stream.len(), STREAM_LEN).into());
    }
    // 88 00 00 00 <u24 BE: bytes after the envelope> 00. For the captured stream: 03 5d e1.
    if stream.get(0..4) != Some(&[0x88,0,0,0][..])
        || stream[7] != 0
        || be24(&stream[4..7]) != stream.len() - 8
    {
        return Err("outer 0x88 envelope mismatch".into());
    }
    if stream.get(8..25) != Some(&[
        0x08,0x05,0x01,0,0x09,0,0xff,0xff,0xff,0,0xff,0xff,0xff,0x02,0x02,0xff,0xff
    ][..]) {
        return Err("upload preamble mismatch".into());
    }
    if stream.get(25..33) != Some(&[0,0x44,0x4c,0x58,0xfc,0xff,0,0][..]) {
        return Err("DLX prefix mismatch".into());
    }
    if stream.get(stream.len()-4..) != Some(&[0xfc,0xff,0,0][..]) {
        return Err("DLX trailer mismatch".into());
    }
    Ok(())
}

pub(crate) fn be24(b: &[u8]) -> usize {
    ((b[0] as usize) << 16) | ((b[1] as usize) << 8) | b[2] as usize
}

/// CONTROL1 announces the DLX file length (u24 BE at bytes 18..21). For the captured
/// 2-frame stream this returns CONTROL1 unchanged.
pub(crate) fn control1_for(stream_len: usize) -> [u8; 24] {
    let file_len = stream_len - FILE_START_IN_STREAM;
    let mut c = CONTROL1;
    c[18] = (file_len >> 16) as u8;
    c[19] = (file_len >> 8) as u8;
    c[20] = file_len as u8;
    c
}

pub(crate) fn build_chunks(stream: &[u8], allow_single_frame: bool) -> Result<Vec<Chunk>, Box<dyn Error>> {
    validate_stream(stream, allow_single_frame)?;
    let mut chunks = Vec::new();
    let mut off = 0usize;

    while off < stream.len() {
        let mut n = 181usize.min(stream.len() - off);

        if off >= FILE_START_IN_STREAM {
            let file_rel = off - FILE_START_IN_STREAM;
            let to_boundary = 4096 - (file_rel % 4096);
            n = n.min(to_boundary);
        }

        let data = &stream[off..off+n];
        let mut value = Vec::with_capacity(n + 1);
        value.extend_from_slice(data);
        value.push(xor8(data));

        let new_off = off + n;
        let ends_4k_block = new_off >= FILE_START_IN_STREAM
            && ((new_off - FILE_START_IN_STREAM) % 4096 == 0);

        chunks.push(Chunk { value, ends_4k_block });
        off = new_off;
    }

    // The captured stream must chunk exactly as PocketWuque did.
    if stream.len() == STREAM_LEN {
        if chunks.len() != CAPTURED_BULK_WRITES {
            return Err(format!("chunk count {} != expected {}", chunks.len(), CAPTURED_BULK_WRITES).into());
        }

        let blocks = chunks.iter().filter(|c| c.ends_4k_block).count();
        if blocks != 53 {
            return Err(format!("detected {blocks} full 4096-byte blocks; expected 53").into());
        }
    }

    Ok(chunks)
}
