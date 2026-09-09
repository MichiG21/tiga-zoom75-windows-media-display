#include <winrt/base.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Media.Control.h>
#include <winrt/Windows.Storage.Streams.h>

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>

using namespace winrt;
using namespace Windows::Media::Control;
using namespace Windows::Storage::Streams;

static std::string json_escape(std::string const& s) {
    std::string out;
    out.reserve(s.size() + 16);
    for (unsigned char c : s) {
        switch (c) {
        case '\"': out += "\\\""; break;
        case '\\': out += "\\\\"; break;
        case '\b': out += "\\b"; break;
        case '\f': out += "\\f"; break;
        case '\n': out += "\\n"; break;
        case '\r': out += "\\r"; break;
        case '\t': out += "\\t"; break;
        default:
            if (c < 0x20) {
                char buf[7];
                snprintf(buf, sizeof(buf), "\\u%04x", c);
                out += buf;
            } else {
                out += static_cast<char>(c);
            }
        }
    }
    return out;
}

static std::string status_name(GlobalSystemMediaTransportControlsSessionPlaybackStatus status) {
    switch (status) {
    case GlobalSystemMediaTransportControlsSessionPlaybackStatus::Closed:   return "Closed";
    case GlobalSystemMediaTransportControlsSessionPlaybackStatus::Opened:   return "Opened";
    case GlobalSystemMediaTransportControlsSessionPlaybackStatus::Changing: return "Changing";
    case GlobalSystemMediaTransportControlsSessionPlaybackStatus::Stopped:  return "Stopped";
    case GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing:  return "Playing";
    case GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused:   return "Paused";
    default: return "Unknown";
    }
}

static bool write_thumbnail(
    GlobalSystemMediaTransportControlsSessionMediaProperties const& props,
    std::filesystem::path const& out_path,
    uint64_t& written_bytes
) {
    written_bytes = 0;
    auto thumb = props.Thumbnail();

    if (!thumb) {
        std::error_code ec;
        std::filesystem::remove(out_path, ec);
        return false;
    }

    auto stream = thumb.OpenReadAsync().get();
    if (!stream) {
        return false;
    }

    uint64_t size64 = stream.Size();
    if (size64 == 0 || size64 > 32ull * 1024ull * 1024ull) {
        return false;
    }

    uint32_t size = static_cast<uint32_t>(size64);
    DataReader reader(stream.GetInputStreamAt(0));
    uint32_t loaded = reader.LoadAsync(size).get();
    if (loaded == 0) {
        return false;
    }

    std::vector<uint8_t> data(loaded);
    reader.ReadBytes(data);

    auto tmp = out_path;
    tmp += L".tmp";

    {
        std::ofstream f(tmp, std::ios::binary | std::ios::trunc);
        if (!f) {
            return false;
        }
        f.write(reinterpret_cast<char const*>(data.data()), static_cast<std::streamsize>(data.size()));
        if (!f) {
            return false;
        }
    }

    std::error_code ec;
    std::filesystem::remove(out_path, ec);
    ec.clear();
    std::filesystem::rename(tmp, out_path, ec);
    if (ec) {
        std::filesystem::remove(tmp, ec);
        return false;
    }

    written_bytes = data.size();
    return true;
}

int wmain(int argc, wchar_t** argv) {
    try {
        init_apartment(apartment_type::multi_threaded);

        std::filesystem::path thumbnail_path = L"windows_media_thumbnail.bin";

        for (int i = 1; i < argc; ++i) {
            std::wstring arg = argv[i];
            if (arg == L"--thumbnail" && i + 1 < argc) {
                thumbnail_path = argv[++i];
            }
        }

        auto manager =
            GlobalSystemMediaTransportControlsSessionManager::RequestAsync().get();

        auto session = manager.GetCurrentSession();
        if (!session) {
            std::cout << R"({"present":false})" << std::endl;
            return 0;
        }

        auto props = session.TryGetMediaPropertiesAsync().get();
        if (!props) {
            std::cout << R"({"present":false})" << std::endl;
            return 0;
        }

        auto playback = session.GetPlaybackInfo();
        auto timeline = session.GetTimelineProperties();

        int64_t duration_ms = 0;
        int64_t position_ms = 0;
        int64_t start_ms = 0;
        int64_t end_ms = 0;
        int64_t min_seek_ms = 0;
        int64_t max_seek_ms = 0;

        if (timeline) {
            position_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                timeline.Position()).count();
            start_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                timeline.StartTime()).count();
            end_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                timeline.EndTime()).count();
            min_seek_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                timeline.MinSeekTime()).count();
            max_seek_ms = std::chrono::duration_cast<std::chrono::milliseconds>(
                timeline.MaxSeekTime()).count();

            // Different media providers populate different timeline fields.
            // Prefer the declared Start/End range, then seek range, then absolute
            // end/max-seek values when the corresponding start is zero/omitted.
            const int64_t declared_span = end_ms - start_ms;
            const int64_t seek_span = max_seek_ms - min_seek_ms;

            if (declared_span > 0) {
                duration_ms = declared_span;
            } else if (seek_span > 0) {
                duration_ms = seek_span;
            } else if (end_ms > 0) {
                duration_ms = end_ms;
            } else if (max_seek_ms > 0) {
                duration_ms = max_seek_ms;
            }
        }

        std::string source = to_string(session.SourceAppUserModelId());
        std::string title = to_string(props.Title());
        std::string artist = to_string(props.Artist());
        std::string album_artist = to_string(props.AlbumArtist());
        std::string album = to_string(props.AlbumTitle());
        std::string subtitle = to_string(props.Subtitle());
        std::string state = status_name(playback.PlaybackStatus());

        uint64_t thumb_bytes = 0;
        bool has_thumb = write_thumbnail(props, thumbnail_path, thumb_bytes);

        std::cout
            << "{"
            << "\"present\":true,"
            << "\"source\":\"" << json_escape(source) << "\","
            << "\"state\":\"" << json_escape(state) << "\","
            << "\"title\":\"" << json_escape(title) << "\","
            << "\"artist\":\"" << json_escape(artist) << "\","
            << "\"album_artist\":\"" << json_escape(album_artist) << "\","
            << "\"album\":\"" << json_escape(album) << "\","
            << "\"subtitle\":\"" << json_escape(subtitle) << "\","
            << "\"track_number\":" << props.TrackNumber() << ","
            << "\"duration_ms\":" << duration_ms << ","
            << "\"position_ms\":" << position_ms << ","
            << "\"timeline_start_ms\":" << start_ms << ","
            << "\"timeline_end_ms\":" << end_ms << ","
            << "\"min_seek_ms\":" << min_seek_ms << ","
            << "\"max_seek_ms\":" << max_seek_ms << ","
            << "\"thumbnail\":" << (has_thumb ? "true" : "false") << ","
            << "\"thumbnail_bytes\":" << thumb_bytes
            << "}"
            << std::endl;

        return 0;
    }
    catch (hresult_error const& e) {
        std::cerr
            << "{\"error\":\"WinRT error 0x"
            << std::hex << static_cast<unsigned long>(e.code().value)
            << ": " << json_escape(to_string(e.message())) << "\"}"
            << std::endl;
        return 1;
    }
    catch (std::exception const& e) {
        std::cerr
            << "{\"error\":\"" << json_escape(e.what()) << "\"}"
            << std::endl;
        return 1;
    }
}
