#include <windows.h>
#include <shlobj.h>
#include <shellapi.h>

#include <filesystem>
#include <fstream>
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

static bool write_command(std::wstring const& command) {
    auto path = command_path();
    if (path.empty()) return false;

    auto tmp = path;
    tmp += L".tmp";

    {
        std::wofstream f(tmp, std::ios::trunc);
        if (!f) return false;
        f << command << L"\n";
        if (!f) return false;
    }

    std::error_code ec;
    std::filesystem::remove(path, ec);
    ec.clear();
    std::filesystem::rename(tmp, path, ec);
    if (ec) {
        std::filesystem::remove(tmp, ec);
        return false;
    }
    return true;
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

    if (!write_command(L"standby")) {
        SetWindowTextW(GetDlgItem(hwnd, 2001), L"Could not request standby; shutdown cancelled.");
        return 0;
    }

    SetWindowTextW(
        GetDlgItem(hwnd, 2001),
        L"Standby requested. Waiting 12 seconds for the BLE upload..."
    );

    Sleep(12000);

    SetWindowTextW(GetDlgItem(hwnd, 2001), L"Shutting down Windows...");
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
            auto* ctx = new ShutdownContext{ hwnd };
            HANDLE thread = CreateThread(nullptr, 0, safe_shutdown_thread, ctx, 0, nullptr);
            if (thread) {
                CloseHandle(thread);
            } else {
                delete ctx;
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
