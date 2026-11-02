//! What the keyboard's media service expects: its characteristics, the two control
//! messages, and the notifications it answers with.

use uuid::Uuid;

pub(crate) const TARGET_NAME: &str = "Zoom75 Tiga";
pub(crate) const CHAR_CONTROL: Uuid = Uuid::from_u128(0x1f400001_aab4_14a3_f1ba_f61f35cddbaa);
pub(crate) const CHAR_NOTIFY: Uuid = Uuid::from_u128(0x1f400002_aab4_14a3_f1ba_f61f35cddbaa);
pub(crate) const CHAR_BULK: Uuid = Uuid::from_u128(0x1f400003_aab4_14a3_f1ba_f61f35cddbaa);
pub(crate) const CHAR_NOTIFY2: Uuid = Uuid::from_u128(0x1f400004_aab4_14a3_f1ba_f61f35cddbaa);

pub(crate) const STREAM_LEN: usize = 220_649;
pub(crate) const FILE_START_IN_STREAM: usize = 25;
pub(crate) const CAPTURED_BULK_WRITES: usize = 1_239;

/// Experimental one-frame containers built by tiga_dlx.py (layouts "index" and
/// "fixed"; see docs/DLX_FORMAT.md). Rejected unless --allow-single-frame is given.
/// Never sent to a keyboard yet.
pub(crate) const SINGLE_FRAME_STREAM_LENS: [usize; 2] = [110_565, 110_569];

pub(crate) const CONTROL1: [u8; 24] = [
    0x88,0,0,0,0,0,0x10,0,
    0x09,0x03,0x04,0,0x08,0,0,0xff,
    0xfc,0,0x03,0x5d,0xd0,0x05,0,0,
];
pub(crate) const CONTROL2: [u8; 18] = [
    0x88,0,0,0,0,0,0x0a,0,
    0x08,0x03,0,0xff,0xff,0xff,0,0xff,0xff,0xff,
];

pub(crate) const C1_ACK_A: &[u8] = &[0x88,0,0,0,0,0,0x05,0x0e,0x07,0x02,0x01,0x09,0x03];
pub(crate) const C1_ACK_B: &[u8] = &[0x88,0,0,0,0,0,0x03,0x0f,0x09,0x04,0x02];
pub(crate) const C2_ACK_A: &[u8] = &[0x88,0,0,0,0,0,0x05,0x0f,0x07,0x02,0x01,0x08,0x03];
pub(crate) const C2_ACK_B: &[u8] = &[0x88,0,0,0,0,0,0x03,0x0e,0x08,0x04,0x02];

pub(crate) const PROGRESS: &[u8] = &[0x88,0,0,0,0,0,0x03,0x0b,0x08,0x06,0x05];
pub(crate) const COMPLETE: &[u8] = &[0x88,0,0,0,0,0,0x03,0x0c,0x08,0x06,0x02];
