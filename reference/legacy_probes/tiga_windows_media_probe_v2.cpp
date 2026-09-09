#include <winrt/base.h>
#include <winrt/Windows.Media.Control.h>

#include <chrono>
#include <iostream>
#include <sstream>
#include <string>
#include <thread>

using namespace winrt;
using namespace Windows::Media::Control;

static std::wstring safe_hstring(winrt::hstring const& s) {
    return std::wstring(s.c_str(), s.size());
}

static wchar_t const* playback_status_name(
    GlobalSystemMediaTransportControlsSessionPlaybackStatus status) {
    switch (status) {
    case GlobalSystemMediaTransportControlsSessionPlaybackStatus::Closed:  return L"Closed";
    case GlobalSystemMediaTransportControlsSessionPlaybackStatus::Opened:  return L"Opened";
    case GlobalSystemMediaTransportControlsSessionPlaybackStatus::Changing:return L"Changing";
    case GlobalSystemMediaTransportControlsSessionPlaybackStatus::Stopped: return L"Stopped";
    case GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing: return L"Playing";
    case GlobalSystemMediaTransportControlsSessionPlaybackStatus::Paused:  return L"Paused";
    default: return L"Unknown";
    }
}

struct MediaSnapshot {
    bool present = false;
    std::wstring source;
    std::wstring state;
    std::wstring title;
    std::wstring artist;
    std::wstring album_artist;
    std::wstring album;
    std::wstring subtitle;
    uint32_t track_number = 0;
    bool thumbnail = false;

    std::wstring key() const {
        if (!present) return L"<none>";
        std::wstringstream ss;
        ss << source << L'|' << state << L'|' << title << L'|' << artist << L'|'
           << album_artist << L'|' << album << L'|' << subtitle << L'|'
           << track_number << L'|' << thumbnail;
        return ss.str();
    }
};

static MediaSnapshot read_current(
    GlobalSystemMediaTransportControlsSessionManager const& manager) {

    MediaSnapshot out{};

    auto session = manager.GetCurrentSession();
    if (!session) {
        return out;
    }

    auto props = session.TryGetMediaPropertiesAsync().get();
    if (!props) {
        return out;
    }

    auto playback = session.GetPlaybackInfo();

    out.present = true;
    out.source = safe_hstring(session.SourceAppUserModelId());
    out.state = playback_status_name(playback.PlaybackStatus());
    out.title = safe_hstring(props.Title());
    out.artist = safe_hstring(props.Artist());
    out.album_artist = safe_hstring(props.AlbumArtist());
    out.album = safe_hstring(props.AlbumTitle());
    out.subtitle = safe_hstring(props.Subtitle());
    out.track_number = props.TrackNumber();
    out.thumbnail = static_cast<bool>(props.Thumbnail());

    return out;
}

static void print_snapshot(MediaSnapshot const& m) {
    std::wcout << L"\n============================================================\n";
    std::wcout << L"TIGA - WINDOWS CURRENT MEDIA SESSION\n";
    std::wcout << L"============================================================\n";

    if (!m.present) {
        std::wcout << L"No current Windows media session.\n";
        return;
    }

    auto show = [](std::wstring const& s) -> std::wstring {
        return s.empty() ? L"(empty)" : s;
    };

    std::wcout << L"Source      : " << show(m.source) << L"\n";
    std::wcout << L"State       : " << show(m.state) << L"\n";
    std::wcout << L"Title       : " << show(m.title) << L"\n";
    std::wcout << L"Artist      : " << show(m.artist) << L"\n";
    std::wcout << L"Album       : " << show(m.album) << L"\n";
    std::wcout << L"AlbumArtist : " << show(m.album_artist) << L"\n";
    std::wcout << L"Subtitle    : " << show(m.subtitle) << L"\n";
    std::wcout << L"Track No.   : " << m.track_number << L"\n";
    std::wcout << L"Thumbnail   : " << (m.thumbnail ? L"YES" : L"NO") << L"\n";
}

int wmain(int argc, wchar_t** argv) {
    try {
        winrt::init_apartment(winrt::apartment_type::multi_threaded);

        bool watch = false;
        int interval_ms = 1000;

        for (int i = 1; i < argc; ++i) {
            std::wstring arg = argv[i];
            if (arg == L"--watch") {
                watch = true;
            } else if (arg == L"--interval-ms" && i + 1 < argc) {
                interval_ms = std::max(100, _wtoi(argv[++i]));
            }
        }

        auto manager =
            GlobalSystemMediaTransportControlsSessionManager::RequestAsync().get();

        if (!watch) {
            print_snapshot(read_current(manager));
            return 0;
        }

        std::wcout << L"Watching Windows current media session every "
                   << interval_ms << L" ms.\n";
        std::wcout << L"Switch between Spotify / YouTube / browser media.\n";
        std::wcout << L"Ctrl+C to stop.\n";

        std::wstring last_key;

        for (;;) {
            auto current = read_current(manager);
            auto key = current.key();

            if (key != last_key) {
                print_snapshot(current);
                last_key = std::move(key);
            }

            std::this_thread::sleep_for(std::chrono::milliseconds(interval_ms));
        }
    }
    catch (winrt::hresult_error const& e) {
        std::wcerr << L"WinRT error 0x" << std::hex
                   << static_cast<unsigned long>(e.code().value)
                   << L": " << e.message().c_str() << L"\n";
        return 1;
    }
    catch (std::exception const& e) {
        std::cerr << "Error: " << e.what() << "\n";
        return 1;
    }
}
