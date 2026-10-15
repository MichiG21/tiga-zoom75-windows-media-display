#include <windows.h>
#include <shlobj.h>
#include <shellapi.h>

#include <filesystem>
#include <fstream>
#include <map>
#include <string>

enum : int {
    ID_STANDBY      = 1001,
    ID_BLACK        = 1002,
    ID_AUTO         = 1003,
    ID_SAFE_SHUTDOWN= 1004,
    ID_CLOSE        = 1005,
};

static std::filesystem::path command_path() {
    wchar_t exe_path[MAX_PATH]{};
    DWORD len = GetModuleFileNameW(nullptr, exe_path, MAX_PATH);
    if (len == 0 || len >= MAX_PATH) {
        return {};
    }
    return std::filesystem::path(exe_path).parent_path() / L"tiga_display_command.txt";
}

// Each write carries a unique ID so the renderer sees a repeated command (Standby
// clicked twice) as new; it used to compare the bare text and ignore it (issue #6).
static std::wstring new_command_id() {
    static unsigned counter = 0;
    FILETIME ft{};
    GetSystemTimeAsFileTime(&ft);
    unsigned long long ticks = (static_cast<unsigned long long>(ft.dwHighDateTime) << 32) | ft.dwLowDateTime;
    return std::to_wstring(ticks) + L"-" + std::to_wstring(GetCurrentProcessId()) + L"-" + std::to_wstring(++counter);
}

// Writes "<command> <id>" and returns that exact text, or an empty string on failure.
static std::wstring write_command_text(std::wstring const& command) {
    auto path = command_path();
    if (path.empty()) return {};

    auto tmp = path;
    tmp += L".tmp";

    std::wstring text = command + L" " + new_command_id();
    {
        std::wofstream f(tmp, std::ios::trunc);
        if (!f) return {};
        f << text << L"\n";
        if (!f) return {};
    }

    std::error_code ec;
    std::filesystem::remove(path, ec);
    ec.clear();
    std::filesystem::rename(tmp, path, ec);
    if (ec) {
        std::filesystem::remove(tmp, ec);
        return {};
    }
    return text;
}

static bool write_command(std::wstring const& command) {
    return !write_command_text(command).empty();
}

// ---- Shutdown acknowledgement (issue #7) ---------------------------------------
//
// The renderer writes tiga_display_command.ack (command_text, crc32 of the frame it
// installed) once it has handled a command. The uploader writes
// extracted\session_01_bulk_stream.bin.delivered (crc32) after the keyboard's
// completion notification. Shutdown waits for both instead of a fixed 12 s.

constexpr DWORD ACK_TIMEOUT_MS      = 15'000;  // renderer polls the command file once a second
constexpr DWORD DELIVERY_TIMEOUT_MS = 90'000;  // an in-flight upload (~8 s) + ours + a reconnect
constexpr DWORD POLL_MS             = 250;

static std::filesystem::path exe_dir() {
    auto path = command_path();
    return path.empty() ? path : path.parent_path();
}

static std::map<std::string, std::string> read_key_values(std::filesystem::path const& path) {
    std::map<std::string, std::string> values;
    std::ifstream f(path);
    std::string line;
    while (std::getline(f, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        auto eq = line.find('=');
        if (eq != std::string::npos) values[line.substr(0, eq)] = line.substr(eq + 1);
    }
    return values;
}

static std::string narrow_ascii(std::wstring const& s) {
    std::string out;
    for (wchar_t c : s) out.push_back(c < 0x80 ? static_cast<char>(c) : '?');
    return out;
}

// Polls until `done` returns true or the timeout elapses.
template <typename F>
static bool wait_until(DWORD timeout_ms, F done) {
    ULONGLONG deadline = GetTickCount64() + timeout_ms;
    while (GetTickCount64() < deadline) {
        if (done()) return true;
        Sleep(POLL_MS);
    }
    return done();
}

static bool ask_shutdown_anyway(HWND hwnd, wchar_t const* problem) {
    std::wstring text = problem;
    text += L"\n\nShutting down now may interrupt an upload, which can leave the keyboard "
            L"needing a power cycle.\n\nShut down anyway?";
    return MessageBoxW(hwnd, text.c_str(), L"TIGA Display Control",
                       MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) == IDYES;
}

static void set_status(HWND hwnd, wchar_t const* text) {
    SetWindowTextW(GetDlgItem(hwnd, 2001), text);
}

struct ShutdownContext {
    HWND hwnd;
};

static DWORD WINAPI safe_shutdown_thread(LPVOID param) {
    auto* ctx = static_cast<ShutdownContext*>(param);
    HWND hwnd = ctx->hwnd;
    delete ctx;

    HWND button = GetDlgItem(hwnd, ID_SAFE_SHUTDOWN);
    auto cancel = [&](wchar_t const* why) {
        SetWindowTextW(GetDlgItem(hwnd, 2001), why);
        EnableWindow(button, TRUE);
        return DWORD{0};
    };

    std::wstring sent = write_command_text(L"standby");
    if (sent.empty()) {
        return cancel(L"Could not request standby; shutdown cancelled.");
    }
    std::string sent_text = narrow_ascii(sent);  // the renderer lower-cases; ours is already

    auto ack_path = exe_dir() / L"tiga_display_command.ack";
    auto delivered_path = exe_dir() / L"extracted" / L"session_01_bulk_stream.bin.delivered";

    // 1. The renderer installed a standby frame for *this* command.
    SetWindowTextW(GetDlgItem(hwnd, 2001), L"Standby requested. Waiting for the renderer...");
    std::string frame_id, crc;
    bool acked = wait_until(ACK_TIMEOUT_MS, [&] {
        auto ack = read_key_values(ack_path);
        if (ack["command_text"] != sent_text) return false;
        frame_id = ack["frame_id"];
        crc = ack["crc32"];
        return !frame_id.empty() && !crc.empty();
    });

    bool delivered = false;
    if (!acked) {
        if (!ask_shutdown_anyway(hwnd, L"The display renderer did not confirm standby. "
                                       L"The TIGA display stack may not be running.")) {
            return cancel(L"Shutdown cancelled: renderer did not confirm standby.");
        }
    } else {
        // 2. The keyboard confirmed that exact frame. Match the frame ID, not only the
        //    CRC: an identical standby frame delivered earlier must not count while a
        //    different frame is mid-upload.
        SetWindowTextW(GetDlgItem(hwnd, 2001),
                       L"Standby frame ready. Waiting for the keyboard to confirm the upload...");
        delivered = wait_until(DELIVERY_TIMEOUT_MS, [&] {
            auto d = read_key_values(delivered_path);
            return d["frame_id"] == frame_id && d["crc32"] == crc;
        });
        if (!delivered &&
            !ask_shutdown_anyway(hwnd, L"The keyboard has not confirmed the standby upload. "
                                       L"It may be out of range, asleep, or still uploading.")) {
            return cancel(L"Shutdown cancelled: upload not confirmed.");
        }
    }

    SetWindowTextW(GetDlgItem(hwnd, 2001),
                   delivered ? L"Standby confirmed on the keyboard. Shutting down Windows..."
                             : L"Shutting down Windows without confirmation...");
    ShellExecuteW(nullptr, L"open", L"shutdown.exe", L"/s /t 0", nullptr, SW_HIDE);
    return 0;
}

static LRESULT CALLBACK WindowProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
    switch (msg) {
    case WM_CREATE:
        CreateWindowW(L"STATIC", L"TIGA Display Control",
            WS_CHILD | WS_VISIBLE,
            20, 16, 300, 24,
            hwnd, nullptr, nullptr, nullptr);

        CreateWindowW(L"BUTTON", L"Lock Standby",
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            20, 54, 150, 34,
            hwnd, reinterpret_cast<HMENU>(ID_STANDBY), nullptr, nullptr);

        CreateWindowW(L"BUTTON", L"Black Screen",
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            185, 54, 150, 34,
            hwnd, reinterpret_cast<HMENU>(ID_BLACK), nullptr, nullptr);

        CreateWindowW(L"BUTTON", L"Resume Automatic",
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            20, 100, 315, 34,
            hwnd, reinterpret_cast<HMENU>(ID_AUTO), nullptr, nullptr);

        CreateWindowW(L"BUTTON", L"Standby + Shut Down",
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            20, 146, 315, 34,
            hwnd, reinterpret_cast<HMENU>(ID_SAFE_SHUTDOWN), nullptr, nullptr);

        CreateWindowW(L"STATIC", L"Ready.",
            WS_CHILD | WS_VISIBLE,
            20, 194, 315, 42,
            hwnd, reinterpret_cast<HMENU>(2001), nullptr, nullptr);

        CreateWindowW(L"BUTTON", L"Close",
            WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
            20, 242, 315, 32,
            hwnd, reinterpret_cast<HMENU>(ID_CLOSE), nullptr, nullptr);
        return 0;

    case WM_COMMAND:
        switch (LOWORD(wParam)) {
        case ID_STANDBY:
            set_status(hwnd, write_command(L"standby")
                ? L"Standby requested."
                : L"Could not write control file.");
            return 0;

        case ID_BLACK:
            set_status(hwnd, write_command(L"black")
                ? L"Black screen requested."
                : L"Could not write control file.");
            return 0;

        case ID_AUTO:
            set_status(hwnd, write_command(L"auto")
                ? L"Automatic media mode requested."
                : L"Could not write control file.");
            return 0;

        case ID_SAFE_SHUTDOWN: {
            // Disabled until the worker cancels; prevents two concurrent shutdown waits.
            EnableWindow(GetDlgItem(hwnd, ID_SAFE_SHUTDOWN), FALSE);
            auto* ctx = new ShutdownContext{ hwnd };
            HANDLE thread = CreateThread(nullptr, 0, safe_shutdown_thread, ctx, 0, nullptr);
            if (thread) {
                CloseHandle(thread);
            } else {
                delete ctx;
                EnableWindow(GetDlgItem(hwnd, ID_SAFE_SHUTDOWN), TRUE);
                set_status(hwnd, L"Could not start safe-shutdown worker.");
            }
            return 0;
        }

        case ID_CLOSE:
            DestroyWindow(hwnd);
            return 0;
        }
        break;

    case WM_DESTROY:
        PostQuitMessage(0);
        return 0;
    }

    return DefWindowProcW(hwnd, msg, wParam, lParam);
}

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE, PWSTR, int show) {
    CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

    wchar_t const CLASS_NAME[] = L"TigaDisplayControlWindow";

    WNDCLASSW wc{};
    wc.lpfnWndProc = WindowProc;
    wc.hInstance = instance;
    wc.lpszClassName = CLASS_NAME;
    wc.hCursor = LoadCursor(nullptr, IDC_ARROW);
    wc.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);

    RegisterClassW(&wc);

    HWND hwnd = CreateWindowExW(
        0,
        CLASS_NAME,
        L"TIGA Display Control",
        WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
        CW_USEDEFAULT, CW_USEDEFAULT,
        375, 335,
        nullptr, nullptr, instance, nullptr);

    if (!hwnd) {
        CoUninitialize();
        return 1;
    }

    ShowWindow(hwnd, show);
    UpdateWindow(hwnd);

    MSG msg{};
    while (GetMessageW(&msg, nullptr, 0, 0) > 0) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    CoUninitialize();
    return 0;
}
