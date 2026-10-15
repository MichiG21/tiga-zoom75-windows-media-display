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

struct WriteResult {
    std::wstring text;   // exactly what was written; empty on failure
    std::wstring error;  // why it failed
};

// True only if the command file holds exactly `text`.
static bool command_file_holds(std::filesystem::path const& path, std::wstring const& text) {
    std::wifstream f(path);
    std::wstring line;
    if (!f || !std::getline(f, line)) return false;
    if (!line.empty() && line.back() == L'\r') line.pop_back();
    return line == text;
}

// Writes "<command> <id>", reads it back, and returns that exact text.
static WriteResult write_command_text(std::wstring const& command) {
    auto path = command_path();
    if (path.empty()) return { {}, L"could not determine this program's folder" };

    auto tmp = path;
    tmp += L".tmp";

    std::wstring text = command + L" " + new_command_id();
    {
        std::wofstream f(tmp, std::ios::trunc);
        if (!f) return { {}, L"could not create " + tmp.wstring() };
        f << text << L"\n";
        if (!f) return { {}, L"could not write " + tmp.wstring() };
    }

    std::error_code ec;
    std::filesystem::remove(path, ec);
    ec.clear();
    std::filesystem::rename(tmp, path, ec);
    if (ec) {
        std::filesystem::remove(tmp, ec);
        return { {}, L"could not move the new file into place" };
    }
    if (!command_file_holds(path, text)) {
        return { {}, L"the file did not contain the command when read back" };
    }
    return { text, {} };
}

static std::wstring write_failure_message(std::wstring const& what, std::wstring const& error) {
    return L"The " + what + L" command was NOT sent: " + error + L".\n\nCommand file:\n" +
           command_path().wstring() +
           L"\n\nIf that folder is writable, antivirus software may be blocking or sandboxing "
           L"this program (Norton CyberCapture, for example, can hold a newly built .exe). "
           L"Try again in a minute, or allow the program in your antivirus.";
}

// A read-back cannot catch everything: a sandboxed program reads back its own
// redirected copy. Only the renderer's acknowledgement proves the command arrived.
static std::wstring renderer_silent_message() {
    return L"The display renderer did not confirm the command.\n\nCommand file:\n" +
           command_path().wstring() +
           L"\n\nEither the display stack is not running (check Status Display, or run Start "
           L"Display), or this program's writes are not reaching that folder. If the file is "
           L"not there, antivirus software may be sandboxing this program (Norton "
           L"CyberCapture, for example, can hold a newly built .exe).";
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

    WriteResult sent = write_command_text(L"standby");
    if (sent.text.empty()) {
        cancel(L"Standby was not sent; shutdown cancelled.");
        MessageBoxW(hwnd, write_failure_message(L"standby", sent.error).c_str(), L"TIGA Display Control",
                    MB_OK | MB_ICONERROR);
        return 0;
    }
    std::string sent_text = narrow_ascii(sent.text);  // the renderer lower-cases; ours is already

    auto ack_path = exe_dir() / L"tiga_display_command.ack";
    auto delivered_path = exe_dir() / L"extracted" / L"session_01_bulk_stream.bin.delivered";

    // 1. The renderer installed a standby frame for *this* command.
    SetWindowTextW(GetDlgItem(hwnd, 2001), L"Standby written. Waiting for the renderer...");
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
        // No shutdown prompt from here: without the renderer's ack, a stopped stack and
        // a command that never arrived look the same.
        cancel(L"Shutdown cancelled: the renderer did not confirm standby.");
        MessageBoxW(hwnd, renderer_silent_message().c_str(), L"TIGA Display Control", MB_OK | MB_ICONWARNING);
        return 0;
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

// ---- The other buttons --------------------------------------------------------------
//
// The status says "written" until the renderer acknowledges the exact command text,
// and says so plainly if it never does.

constexpr UINT_PTR ACK_TIMER             = 1;
constexpr DWORD    BUTTON_ACK_TIMEOUT_MS = 10'000;

static std::string  g_pending;       // command text awaiting the renderer's ack
static std::wstring g_pending_done;  // status to show once acknowledged
static ULONGLONG    g_pending_deadline = 0;

static void send_command(HWND hwnd, wchar_t const* command, wchar_t const* label, wchar_t const* done) {
    KillTimer(hwnd, ACK_TIMER);
    g_pending.clear();

    WriteResult r = write_command_text(command);
    if (r.text.empty()) {
        set_status(hwnd, L"Not sent: could not write the control file.");
        MessageBoxW(hwnd, write_failure_message(label, r.error).c_str(), L"TIGA Display Control",
                    MB_OK | MB_ICONERROR);
        return;
    }
    g_pending = narrow_ascii(r.text);
    g_pending_done = done;
    g_pending_deadline = GetTickCount64() + BUTTON_ACK_TIMEOUT_MS;
    set_status(hwnd, (std::wstring(L"Written. Waiting for the renderer to confirm ") + label + L"...").c_str());
    SetTimer(hwnd, ACK_TIMER, POLL_MS, nullptr);
}

static void check_pending_ack(HWND hwnd) {
    if (g_pending.empty()) {
        KillTimer(hwnd, ACK_TIMER);
        return;
    }
    if (read_key_values(exe_dir() / L"tiga_display_command.ack")["command_text"] == g_pending) {
        KillTimer(hwnd, ACK_TIMER);
        g_pending.clear();
        set_status(hwnd, g_pending_done.c_str());
    } else if (GetTickCount64() >= g_pending_deadline) {
        KillTimer(hwnd, ACK_TIMER);  // before the message box, whose loop would keep it firing
        g_pending.clear();
        set_status(hwnd, L"The renderer did not confirm the command.");
        MessageBoxW(hwnd, renderer_silent_message().c_str(), L"TIGA Display Control", MB_OK | MB_ICONWARNING);
    }
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
            send_command(hwnd, L"standby", L"standby",
                         L"Standby confirmed by the renderer; uploading to the keyboard.");
            return 0;

        case ID_BLACK:
            send_command(hwnd, L"black", L"black screen",
                         L"Black screen confirmed by the renderer; uploading to the keyboard.");
            return 0;

        case ID_AUTO:
            send_command(hwnd, L"auto", L"automatic mode", L"Automatic mode confirmed by the renderer.");
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

    case WM_TIMER:
        if (wParam == ACK_TIMER) {
            check_pending_ack(hwnd);
            return 0;
        }
        break;

    case WM_DESTROY:
        KillTimer(hwnd, ACK_TIMER);
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
