// JanScope ランチャー（JanScope.exe）: サーバー・配信クライアント・入力担当・選手用画面を1か所から起動する。
//
// - Win32 API だけで作る（ライブラリなし）。メニューバー・ツールバー・ステータスバー・中央にバージョン情報
// - 更新は janscope-server.exe の --update-info / --update に任せる（SHA-256 の確認・失敗時の巻き戻しは Rust 側）
// - 接続の設定（サーバーのアドレス・ポート・接続コード・選手用画面のモニター）は JanScope.ini に保存する。
//   このファイルは配布 zip に入れないので、更新で消えない
// - このPCでサーバーを起動するときは、設定の接続コードで起動するので、クライアント側でコードを打たなくてよい
//
// ビルド: build.bat（Visual Studio）または
//   x86_64-w64-mingw32-windres JanScope.rc -O coff -o JanScope.res.o
//   x86_64-w64-mingw32-g++ -O2 -municode -mwindows -static -DJANSCOPE_VERSION="\"x.y.z\"" JanScope.cpp JanScope.res.o -lcomctl32 -lshell32 -lshlwapi -o JanScope.exe

#ifndef UNICODE
#define UNICODE
#endif
#ifndef _UNICODE
#define _UNICODE
#endif
#ifndef _WIN32_WINNT
#define _WIN32_WINNT 0x0A00
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif

#include <windows.h>
#include <commctrl.h>
#include <shellapi.h>
#include <shlwapi.h>

#include <algorithm>
#include <cstdint>
#include <random>
#include <string>
#include <thread>
#include <vector>

#ifndef JANSCOPE_VERSION
#define JANSCOPE_VERSION "dev"
#endif

static const wchar_t* kRepoUrl = L"https://github.com/sarumogu2013-glitch/MJview";
static const wchar_t* kReleasesUrl = L"https://github.com/sarumogu2013-glitch/MJview/releases";

// ---- コマンド ID ----
enum : UINT {
    ID_SERVER = 1001,
    ID_SERVER_RESUME,
    ID_BROADCAST,
    ID_INPUT,
    ID_PLAYER,
    ID_DEMO,
    ID_STOP_ALL,
    ID_SETTINGS,
    ID_EXIT,
    ID_UPDATE_CHECK,
    ID_UPDATE_APPLY,
    ID_UPDATE_ON_START,
    ID_RELEASES,
    ID_OPEN_FOLDER,
    ID_OPEN_DATA,
    ID_OPEN_FFMPEG,
    ID_README,
    ID_LICENSES,
    ID_SOURCE,
    ID_ABOUT,
    ID_VIEW_INFO,
    ID_VIEW_BROADCAST,
    ID_VIEW_INPUT,
    ID_VIEW_DEMO,
};

enum : UINT {
    WM_APP_UPDATE_INFO = WM_APP + 1,  // lParam: UpdateInfo*
    WM_APP_UPDATE_DONE,               // wParam: 終了コード, lParam: std::wstring*（出力）
};

enum { TIMER_CHILDREN = 1, TIMER_EMBED = 2 };

// ---- 文字列 ----
static std::wstring Widen(const std::string& s) {
    if (s.empty()) return L"";
    int n = MultiByteToWideChar(CP_UTF8, 0, s.data(), (int)s.size(), nullptr, 0);
    std::wstring w(n, L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s.data(), (int)s.size(), w.data(), n);
    return w;
}

static std::wstring Trim(std::wstring s) {
    auto sp = [](wchar_t c) { return c == L' ' || c == L'\t' || c == L'\r' || c == L'\n' || c == 0xFEFF; };
    while (!s.empty() && sp(s.back())) s.pop_back();
    size_t i = 0;
    while (i < s.size() && sp(s[i])) i++;
    return s.substr(i);
}

// ---- 場所 ----
static std::wstring ExeDir() {
    wchar_t buf[MAX_PATH * 2];
    DWORD n = GetModuleFileNameW(nullptr, buf, (DWORD)(sizeof(buf) / sizeof(buf[0])));
    std::wstring p(buf, n);
    size_t k = p.find_last_of(L"\\/");
    return k == std::wstring::npos ? L"." : p.substr(0, k);
}
static std::wstring ExePath() {
    wchar_t buf[MAX_PATH * 2];
    DWORD n = GetModuleFileNameW(nullptr, buf, (DWORD)(sizeof(buf) / sizeof(buf[0])));
    return std::wstring(buf, n);
}
static std::wstring InDir(const std::wstring& name) { return ExeDir() + L"\\" + name; }
static bool Exists(const std::wstring& p) { return GetFileAttributesW(p.c_str()) != INVALID_FILE_ATTRIBUTES; }

/// 配布フォルダの VERSION.txt、なければビルド時の版
static std::wstring Version() {
    HANDLE f = CreateFileW(InDir(L"VERSION.txt").c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr, OPEN_EXISTING, 0, nullptr);
    if (f != INVALID_HANDLE_VALUE) {
        char buf[64] = {};
        DWORD got = 0;
        ReadFile(f, buf, sizeof(buf) - 1, &got, nullptr);
        CloseHandle(f);
        std::wstring v = Trim(Widen(std::string(buf, got)));
        if (!v.empty()) return v;
    }
    return Widen(JANSCOPE_VERSION);
}

// ---- 設定（JanScope.ini）----
struct Settings {
    std::wstring host = L"127.0.0.1";
    std::wstring port = L"47800";
    std::wstring code;
    std::wstring playerMonitor = L"1";
    bool playerFullscreen = true;
    bool checkOnStart = true;
};

static std::wstring IniPath() { return InDir(L"JanScope.ini"); }

static std::wstring IniGet(const wchar_t* key, const std::wstring& def) {
    wchar_t buf[512];
    GetPrivateProfileStringW(L"JanScope", key, def.c_str(), buf, 512, IniPath().c_str());
    return Trim(buf);
}
static void IniSet(const wchar_t* key, const std::wstring& v) {
    WritePrivateProfileStringW(L"JanScope", key, v.c_str(), IniPath().c_str());
}

static Settings LoadSettings() {
    Settings s;
    s.host = IniGet(L"server_host", s.host);
    s.port = IniGet(L"server_port", s.port);
    s.code = IniGet(L"code", L"");
    s.playerMonitor = IniGet(L"player_monitor", s.playerMonitor);
    s.playerFullscreen = IniGet(L"player_fullscreen", L"1") != L"0";
    s.checkOnStart = IniGet(L"check_update_on_start", L"1") != L"0";
    return s;
}
static void SaveSettings(const Settings& s) {
    IniSet(L"server_host", s.host);
    IniSet(L"server_port", s.port);
    IniSet(L"code", s.code);
    IniSet(L"player_monitor", s.playerMonitor);
    IniSet(L"player_fullscreen", s.playerFullscreen ? L"1" : L"0");
    IniSet(L"check_update_on_start", s.checkOnStart ? L"1" : L"0");
}

static std::wstring NewCode() {
    std::random_device rd;
    std::uniform_int_distribution<int> d(0, 999999);
    wchar_t buf[8];
    swprintf(buf, 8, L"%06d", d(rd));
    return buf;
}

static bool ValidCode(const std::wstring& c) {
    return c.size() == 6 && std::all_of(c.begin(), c.end(), [](wchar_t ch) { return ch >= L'0' && ch <= L'9'; });
}
static bool ValidPort(const std::wstring& p) {
    if (p.empty() || p.size() > 5 || !std::all_of(p.begin(), p.end(), [](wchar_t ch) { return ch >= L'0' && ch <= L'9'; })) return false;
    int v = _wtoi(p.c_str());
    return v >= 1 && v <= 65535;
}

// ---- 起動したもの ----
// 各機能（サーバー・配信クライアント・入力担当・選手用画面・画面の確認）は1つずつ。
// ツールバーのボタンで起動し、動いている間はボタンが押された状態。もう一度押すと終了する
struct Child {
    std::wstring name;
    HANDLE process;
    DWORD pid;
    UINT fn;                  // どの機能か（ID_SERVER など）
    HWND embedded = nullptr;  // メインウィンドウに埋め込んだ操作ウィンドウ
    bool waitEmbed = false;   // 埋め込む操作ウィンドウが出るのを待っている
    ULONGLONG waitUntil = 0;
};

static HINSTANCE g_inst;
static HWND g_main, g_status, g_tips;
// ツールバー: 文字だけの普通のボタンを並べる（画像やコモンコントロールのツールバーは使わない）
static std::vector<HWND> g_buttons;
static int g_barH = 0;
// いまメインウィンドウに見せている操作ウィンドウ（埋め込みは複数できる。見せるのは1つ）
static HWND g_embedded = nullptr;
static HFONT g_font, g_fontTitle, g_fontSmall;
static Settings g_set;
static std::vector<Child> g_children;
static std::wstring g_version;

struct UpdateInfo {
    bool ok = false;  // 情報を取れた
    bool available = false;
    bool hasFile = false;
    bool dist = false;
    std::wstring current, latest, notes, url, error;
};
static UpdateInfo g_update;
static bool g_updateChecking = false;
static bool g_updating = false;

static void SetStatus(int part, const std::wstring& text) {
    SendMessageW(g_status, SB_SETTEXTW, part, (LPARAM)text.c_str());
}

static void Error(const std::wstring& msg) { MessageBoxW(g_main, msg.c_str(), L"JanScope", MB_OK | MB_ICONWARNING); }

static Child* Running(UINT fn) {
    for (auto& c : g_children)
        if (c.fn == fn && WaitForSingleObject(c.process, 0) == WAIT_TIMEOUT) return &c;
    return nullptr;
}

/// exe を起動する。console=true ならコンソール（サーバーの接続コードなどが見える窓）を出す
static Child* Launch(UINT fn, const std::wstring& title, const std::wstring& exe, const std::wstring& args, bool console) {
    std::wstring path = InDir(exe);
    if (!Exists(path)) {
        Error(exe + L" が見つかりません。\nJanScope.exe と同じフォルダに置いてください。\n\n" + path);
        return nullptr;
    }
    std::wstring cmd = L"\"" + path + L"\" " + args;
    STARTUPINFOW si{};
    si.cb = sizeof(si);
    PROCESS_INFORMATION pi{};
    DWORD flags = console ? CREATE_NEW_CONSOLE : CREATE_NO_WINDOW;
    std::wstring dir = ExeDir();
    if (!CreateProcessW(nullptr, cmd.data(), nullptr, nullptr, FALSE, flags, nullptr, dir.c_str(), &si, &pi)) {
        wchar_t buf[64];
        swprintf(buf, 64, L"（エラー %lu）", GetLastError());
        Error(title + L" を起動できませんでした" + buf);
        return nullptr;
    }
    CloseHandle(pi.hThread);
    Child c;
    c.name = title;
    c.process = pi.hProcess;
    c.pid = pi.dwProcessId;
    c.fn = fn;
    g_children.push_back(c);
    return &g_children.back();
}

// ---- 操作ウィンドウの埋め込み ----
struct FindCtx {
    DWORD pid;
    HWND found;
};
static BOOL CALLBACK FindOperator(HWND h, LPARAM l) {
    auto* c = (FindCtx*)l;
    DWORD pid = 0;
    GetWindowThreadProcessId(h, &pid);
    if (pid != c->pid || !IsWindowVisible(h)) return TRUE;
    wchar_t t[128];
    GetWindowTextW(h, t, 128);
    if (wcsncmp(t, L"JanScope 操作", 11) == 0) {
        c->found = h;
        return FALSE;
    }
    return TRUE;
}

static RECT ContentRect() {
    RECT rc;
    GetClientRect(g_main, &rc);
    RECT sb;
    GetWindowRect(g_status, &sb);
    rc.top += g_barH;
    rc.bottom -= sb.bottom - sb.top;
    if (rc.bottom < rc.top) rc.bottom = rc.top;
    return rc;
}

static void FitEmbedded() {
    if (!g_embedded) return;
    RECT rc = ContentRect();
    SetWindowPos(g_embedded, HWND_TOP, rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, SWP_NOACTIVATE | SWP_SHOWWINDOW);
}

/// 埋め込んだ操作ウィンドウのうち、これを見せる（ほかは隠す）。nullptr ならバージョン情報を見せる
static void ShowEmbedded(HWND op) {
    for (auto& c : g_children)
        if (c.embedded && c.embedded != op && IsWindow(c.embedded)) ShowWindow(c.embedded, SW_HIDE);
    g_embedded = op;
    if (op) {
        FitEmbedded();
        SetFocus(op);
    }
    InvalidateRect(g_main, nullptr, TRUE);
}

static void Embed(Child& c, HWND op) {
    LONG_PTR st = GetWindowLongPtrW(op, GWL_STYLE);
    st &= ~(WS_POPUP | WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX);
    st |= WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS;
    SetWindowLongPtrW(op, GWL_STYLE, st);
    LONG_PTR ex = GetWindowLongPtrW(op, GWL_EXSTYLE);
    ex &= ~(WS_EX_APPWINDOW | WS_EX_DLGMODALFRAME | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE);
    SetWindowLongPtrW(op, GWL_EXSTYLE, ex);
    SetParent(op, g_main);
    c.embedded = op;
    c.waitEmbed = false;
    // 操作画面が入る大きさにする（作業領域いっぱい）
    if (!IsZoomed(g_main)) ShowWindow(g_main, SW_MAXIMIZE);
    SetWindowPos(op, nullptr, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
    ShowEmbedded(op);
    SetStatus(1, c.name + L" の操作画面をこのウィンドウに出しています（切替は「表示」メニュー）");
}

static void OnEmbedTimer() {
    bool waiting = false;
    for (auto& c : g_children) {
        if (!c.waitEmbed) continue;
        FindCtx f{c.pid, nullptr};
        EnumWindows(FindOperator, (LPARAM)&f);
        if (f.found) {
            Embed(c, f.found);
            continue;
        }
        bool alive = WaitForSingleObject(c.process, 0) == WAIT_TIMEOUT;
        if (!alive || GetTickCount64() > c.waitUntil) {
            c.waitEmbed = false;
            SetStatus(1, alive ? c.name + L" の操作画面を埋め込めませんでした（別のウィンドウのままです）" : c.name + L" が終了しました");
            continue;
        }
        waiting = true;
    }
    if (!waiting) KillTimer(g_main, TIMER_EMBED);
}

// ---- ボタンの押された状態 ----
static const UINT kToggles[] = {ID_SERVER, ID_BROADCAST, ID_INPUT, ID_PLAYER, ID_DEMO};

static void UpdateToggleButtons() {
    for (UINT fn : kToggles) {
        HWND b = GetDlgItem(g_main, (int)fn);
        if (b) SendMessageW(b, BM_SETCHECK, Running(fn) ? BST_CHECKED : BST_UNCHECKED, 0);
    }
}

static void RefreshChildren() {
    bool shownGone = false;
    g_children.erase(std::remove_if(g_children.begin(), g_children.end(),
                                    [&](const Child& c) {
                                        if (WaitForSingleObject(c.process, 0) == WAIT_OBJECT_0) {
                                            if (c.embedded && c.embedded == g_embedded) shownGone = true;
                                            CloseHandle(c.process);
                                            return true;
                                        }
                                        return false;
                                    }),
                     g_children.end());
    if (g_embedded && !IsWindow(g_embedded)) shownGone = true;
    if (shownGone) {
        // 見せていた操作画面が終わった: 残っている埋め込みのうち最後のもの、なければバージョン情報
        HWND next = nullptr;
        for (auto& c : g_children)
            if (c.embedded && IsWindow(c.embedded)) next = c.embedded;
        ShowEmbedded(next);
        if (!next) SetStatus(1, L"");
    }
    std::wstring t;
    if (g_children.empty()) {
        t = L"起動中のもの: なし";
    } else {
        t = L"起動中: ";
        for (size_t i = 0; i < g_children.size(); i++) {
            if (i) t += L"・";
            t += g_children[i].name;
        }
    }
    SetStatus(0, t);
    UpdateToggleButtons();
}

static void Stop(Child* c) {
    TerminateProcess(c->process, 0);
    WaitForSingleObject(c->process, 2000);
    RefreshChildren();
}

static void StopAll() {
    if (g_children.empty()) return;
    if (MessageBoxW(g_main, L"起動したもの（サーバー・クライアントなど）をすべて閉じますか？\n配信中・対局中は閉じないでください。",
                    L"JanScope", MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES)
        return;
    for (auto& c : g_children) TerminateProcess(c.process, 0);
    Sleep(300);
    RefreshChildren();
}

static std::wstring ServerAddr() { return g_set.host + L":" + g_set.port; }

static bool NeedCode() {
    if (ValidCode(g_set.code)) return true;
    Error(L"接続コード（6桁の数字）が設定されていません。\n「ファイル → 接続の設定」で入れてください。\n（このPCでサーバーを起動すると、自動で作ります）");
    return false;
}

static void StartServer(bool resume) {
    if (!ValidCode(g_set.code)) {
        g_set.code = NewCode();
        SaveSettings(g_set);
    }
    if (!ValidPort(g_set.port)) {
        Error(L"ポートの番号が正しくありません。「ファイル → 接続の設定」で直してください。");
        return;
    }
    std::wstring args = L"--port " + g_set.port + L" --code " + g_set.code;
    if (resume) args += L" --resume-latest";
    Launch(ID_SERVER, resume ? L"サーバー（再開）" : L"サーバー", L"janscope-server.exe", args, true);
    InvalidateRect(g_main, nullptr, TRUE);
}

static void StartClient(UINT fn) {
    std::wstring args, title = L"配信クライアント";
    if (fn == ID_DEMO) {
        args = L"--demo";
        title = L"画面の確認";
    } else {
        if (!NeedCode()) return;
        args = L"--server " + ServerAddr() + L" --code " + g_set.code;
    }
    if (fn == ID_INPUT) {
        args += L" --role input";
        title = L"入力担当";
    } else if (fn == ID_PLAYER) {
        args += L" --role player";
        if (g_set.playerFullscreen) args += L" --fullscreen --monitor " + g_set.playerMonitor;
        title = L"選手用画面";
    }
    // 操作ウィンドウはメインウィンドウに埋め込む（選手用画面は選手側のモニターに出すので別ウィンドウ）
    bool embed = fn != ID_PLAYER;
    if (embed) args += L" --embed";
    Child* c = Launch(fn, title, L"janscope-client.exe", args, false);
    if (c && embed) {
        c->waitEmbed = true;
        c->waitUntil = GetTickCount64() + 30000;
        SetTimer(g_main, TIMER_EMBED, 100, nullptr);
        SetStatus(1, title + L" の操作画面を開いています…");
    }
}

/// ボタン: 動いていなければ起動、動いていれば終了（サーバーと配信クライアントは確認する）
static void Toggle(UINT fn) {
    if (Child* c = Running(fn)) {
        if (fn == ID_SERVER || fn == ID_BROADCAST) {
            std::wstring q = c->name + L" を終了しますか？\n配信中・対局中は終了しないでください。";
            if (MessageBoxW(g_main, q.c_str(), L"JanScope", MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES) {
                UpdateToggleButtons();
                return;
            }
        }
        Stop(c);
        return;
    }
    if (fn == ID_SERVER) StartServer(false);
    else StartClient(fn);
    UpdateToggleButtons();
}

/// 「表示」メニュー: 埋め込んだ操作画面を切り替える
static void ViewEmbedded(UINT fn) {
    Child* c = Running(fn);
    if (c && c->embedded && IsWindow(c->embedded)) ShowEmbedded(c->embedded);
}

// ---- 外部プログラムを動かして出力を受け取る（更新用）----
static bool RunCapture(const std::wstring& args, std::string& out, DWORD& code) {
    std::wstring path = InDir(L"janscope-server.exe");
    if (!Exists(path)) {
        out = "janscope-server.exe が見つかりません";
        code = 1;
        return false;
    }
    SECURITY_ATTRIBUTES sa{sizeof(sa), nullptr, TRUE};
    HANDLE rd = nullptr, wr = nullptr;
    if (!CreatePipe(&rd, &wr, &sa, 0)) return false;
    SetHandleInformation(rd, HANDLE_FLAG_INHERIT, 0);
    STARTUPINFOW si{};
    si.cb = sizeof(si);
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdOutput = wr;
    si.hStdError = wr;
    si.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
    PROCESS_INFORMATION pi{};
    std::wstring cmd = L"\"" + path + L"\" " + args;
    std::wstring dir = ExeDir();
    BOOL ok = CreateProcessW(nullptr, cmd.data(), nullptr, nullptr, TRUE, CREATE_NO_WINDOW, nullptr, dir.c_str(), &si, &pi);
    CloseHandle(wr);
    if (!ok) {
        CloseHandle(rd);
        out = "janscope-server.exe を起動できません";
        code = 1;
        return false;
    }
    char buf[4096];
    DWORD n;
    while (ReadFile(rd, buf, sizeof(buf), &n, nullptr) && n > 0) out.append(buf, n);
    CloseHandle(rd);
    WaitForSingleObject(pi.hProcess, INFINITE);
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);
    return true;
}

// ---- 小さな JSON の読み取り（--update-info の1行だけ読めればよい）----
static size_t JsonFindKey(const std::string& j, const std::string& key) {
    std::string k = "\"" + key + "\"";
    size_t p = j.find(k);
    if (p == std::string::npos) return p;
    p = j.find(':', p + k.size());
    if (p == std::string::npos) return p;
    p++;
    while (p < j.size() && (j[p] == ' ' || j[p] == '\t')) p++;
    return p;
}
static bool JsonBool(const std::string& j, const std::string& key) {
    size_t p = JsonFindKey(j, key);
    return p != std::string::npos && j.compare(p, 4, "true") == 0;
}
static void AppendUtf8(std::string& s, uint32_t cp) {
    if (cp < 0x80) s += (char)cp;
    else if (cp < 0x800) { s += (char)(0xC0 | (cp >> 6)); s += (char)(0x80 | (cp & 0x3F)); }
    else if (cp < 0x10000) { s += (char)(0xE0 | (cp >> 12)); s += (char)(0x80 | ((cp >> 6) & 0x3F)); s += (char)(0x80 | (cp & 0x3F)); }
    else { s += (char)(0xF0 | (cp >> 18)); s += (char)(0x80 | ((cp >> 12) & 0x3F)); s += (char)(0x80 | ((cp >> 6) & 0x3F)); s += (char)(0x80 | (cp & 0x3F)); }
}
static std::wstring JsonString(const std::string& j, const std::string& key) {
    size_t p = JsonFindKey(j, key);
    if (p == std::string::npos || p >= j.size() || j[p] != '"') return L"";
    std::string s;
    for (size_t i = p + 1; i < j.size(); i++) {
        char c = j[i];
        if (c == '"') break;
        if (c == '\\' && i + 1 < j.size()) {
            char e = j[++i];
            switch (e) {
                case 'n': s += "\r\n"; break;
                case 'r': break;
                case 't': s += '\t'; break;
                case 'u':
                    if (i + 4 < j.size()) {
                        uint32_t cp = (uint32_t)strtoul(j.substr(i + 1, 4).c_str(), nullptr, 16);
                        i += 4;
                        // サロゲートペア
                        if (cp >= 0xD800 && cp <= 0xDBFF && i + 6 < j.size() && j[i + 1] == '\\' && j[i + 2] == 'u') {
                            uint32_t lo = (uint32_t)strtoul(j.substr(i + 3, 4).c_str(), nullptr, 16);
                            cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                            i += 6;
                        }
                        AppendUtf8(s, cp);
                    }
                    break;
                default: s += e; break;
            }
        } else {
            s += c;
        }
    }
    return Widen(s);
}

static void CheckUpdateAsync(bool manual) {
    if (g_updateChecking || g_updating) return;
    g_updateChecking = true;
    SetStatus(1, L"新しい版を確かめています…");
    HWND hwnd = g_main;
    std::thread([hwnd, manual] {
        std::string out;
        DWORD code = 0;
        auto* info = new UpdateInfo();
        if (RunCapture(L"--update-info", out, code)) {
            // 最後の { から始まる行を読む
            size_t b = out.rfind('{');
            std::string j = b == std::string::npos ? "" : out.substr(b);
            info->ok = !j.empty();
            info->available = JsonBool(j, "available");
            info->hasFile = JsonBool(j, "has_file");
            info->dist = JsonBool(j, "dist");
            info->current = JsonString(j, "current");
            info->latest = JsonString(j, "latest");
            info->notes = JsonString(j, "notes");
            info->url = JsonString(j, "url");
            info->error = JsonString(j, "error");
            if (!info->ok) info->error = Widen(out);
        } else {
            info->error = Widen(out);
        }
        PostMessageW(hwnd, WM_APP_UPDATE_INFO, manual ? 1 : 0, (LPARAM)info);
    }).detach();
}

static void ApplyUpdate() {
    if (g_updating) return;
    if (!g_update.available) {
        CheckUpdateAsync(true);
        return;
    }
    if (!g_update.dist) {
        Error(L"開発中の実行（配布フォルダではない）なので、入れ替えはできません。");
        return;
    }
    if (!g_children.empty()) {
        Error(L"サーバー・クライアントなどが起動中です。\n配信・対局が終わってから、すべて閉じて更新してください。\n（「起動 → 起動したものをすべて閉じる」）");
        return;
    }
    std::wstring msg = L"新しい版 " + g_update.latest + L" に入れ替えます（今は " + g_update.current +
                       L"）。\n\n設定・選手・対局ログ・ffmpeg・JanScope.ini はそのまま残ります。\n配信中・対局中は入れ替えないでください。\n\n入れ替えますか？";
    if (MessageBoxW(g_main, msg.c_str(), L"JanScope の更新", MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2) != IDYES) return;
    g_updating = true;
    SetStatus(1, L"新しい版を取ってきています…（しばらくかかります）");
    for (HWND b : g_buttons) EnableWindow(b, FALSE);
    HWND hwnd = g_main;
    std::thread([hwnd] {
        std::string out;
        DWORD code = 1;
        RunCapture(L"--update", out, code);
        PostMessageW(hwnd, WM_APP_UPDATE_DONE, code, (LPARAM) new std::wstring(Widen(out)));
    }).detach();
}

static void Open(const std::wstring& target) {
    ShellExecuteW(g_main, L"open", target.c_str(), nullptr, nullptr, SW_SHOWNORMAL);
}

static void About() {
    std::wstring t = L"JanScope " + g_version +
                     L"\n麻雀大会の中継システム\n\n"
                     L"このソフトウェアは AI（Anthropic の Claude）で作成しました。\n"
                     L"人による網羅的なコードレビューは行っていません。大会など本番で使う前に、必ず実際の環境で動作と計算結果を確かめてください。\n\n"
                     L"ライセンス: MIT OR Apache-2.0（同梱物は THIRD_PARTY_LICENSES.md）\n"
                     L"ソースコード: " +
                     std::wstring(kRepoUrl);
    MessageBoxW(g_main, t.c_str(), L"バージョン情報", MB_OK | MB_ICONINFORMATION);
}

// ---- 接続の設定ウィンドウ ----
static HWND g_setWnd;
enum { IDC_HOST = 2001, IDC_PORT, IDC_CODE, IDC_NEWCODE, IDC_MON, IDC_FULL, IDC_ONSTART, IDC_OK, IDC_CANCEL };

static HWND AddCtl(HWND parent, const wchar_t* cls, const wchar_t* text, DWORD style, int x, int y, int w, int h, int id, DWORD ex = 0) {
    UINT dpi = GetDpiForWindow(parent);
    auto s = [dpi](int v) { return MulDiv(v, dpi, 96); };
    HWND c = CreateWindowExW(ex, cls, text, WS_CHILD | WS_VISIBLE | style, s(x), s(y), s(w), s(h), parent, (HMENU)(INT_PTR)id, g_inst, nullptr);
    SendMessageW(c, WM_SETFONT, (WPARAM)g_font, TRUE);
    return c;
}

static std::wstring GetText(HWND dlg, int id) {
    wchar_t buf[256];
    GetDlgItemTextW(dlg, id, buf, 256);
    return Trim(buf);
}

static void CloseSettings() {
    EnableWindow(g_main, TRUE);
    DestroyWindow(g_setWnd);
    g_setWnd = nullptr;
    SetForegroundWindow(g_main);
}

static LRESULT CALLBACK SettingsProc(HWND h, UINT m, WPARAM w, LPARAM l) {
    switch (m) {
        case WM_CREATE: {
            int y = 16;
            AddCtl(h, L"STATIC", L"サーバーのアドレス（このPCなら 127.0.0.1）", 0, 16, y, 300, 20, -1);
            AddCtl(h, L"EDIT", g_set.host.c_str(), WS_BORDER | ES_AUTOHSCROLL | WS_TABSTOP, 16, y + 22, 220, 24, IDC_HOST);
            AddCtl(h, L"STATIC", L"ポート", 0, 250, y, 80, 20, -1);
            AddCtl(h, L"EDIT", g_set.port.c_str(), WS_BORDER | ES_NUMBER | WS_TABSTOP, 250, y + 22, 80, 24, IDC_PORT);
            y += 60;
            AddCtl(h, L"STATIC", L"接続コード（6桁。サーバーとクライアントで同じにする）", 0, 16, y, 340, 20, -1);
            AddCtl(h, L"EDIT", g_set.code.c_str(), WS_BORDER | ES_NUMBER | WS_TABSTOP, 16, y + 22, 120, 24, IDC_CODE);
            AddCtl(h, L"BUTTON", L"新しく作る", BS_PUSHBUTTON | WS_TABSTOP, 146, y + 21, 100, 26, IDC_NEWCODE);
            y += 60;
            AddCtl(h, L"STATIC", L"選手用画面を出すモニター（0 から数える）", 0, 16, y, 300, 20, -1);
            AddCtl(h, L"EDIT", g_set.playerMonitor.c_str(), WS_BORDER | ES_NUMBER | WS_TABSTOP, 16, y + 22, 60, 24, IDC_MON);
            AddCtl(h, L"BUTTON", L"全画面で出す", BS_AUTOCHECKBOX | WS_TABSTOP, 90, y + 22, 200, 24, IDC_FULL);
            CheckDlgButton(h, IDC_FULL, g_set.playerFullscreen ? BST_CHECKED : BST_UNCHECKED);
            y += 60;
            AddCtl(h, L"BUTTON", L"起動時に新しい版があるか確かめる（入れ替えは手動）", BS_AUTOCHECKBOX | WS_TABSTOP, 16, y, 340, 24, IDC_ONSTART);
            CheckDlgButton(h, IDC_ONSTART, g_set.checkOnStart ? BST_CHECKED : BST_UNCHECKED);
            y += 40;
            AddCtl(h, L"BUTTON", L"保存", BS_DEFPUSHBUTTON | WS_TABSTOP, 160, y, 90, 30, IDC_OK);
            AddCtl(h, L"BUTTON", L"キャンセル", BS_PUSHBUTTON | WS_TABSTOP, 260, y, 90, 30, IDC_CANCEL);
            return 0;
        }
        case WM_COMMAND:
            switch (LOWORD(w)) {
                case IDC_NEWCODE: SetDlgItemTextW(h, IDC_CODE, NewCode().c_str()); return 0;
                case IDC_OK: {
                    Settings s = g_set;
                    s.host = GetText(h, IDC_HOST);
                    s.port = GetText(h, IDC_PORT);
                    s.code = GetText(h, IDC_CODE);
                    s.playerMonitor = GetText(h, IDC_MON);
                    s.playerFullscreen = IsDlgButtonChecked(h, IDC_FULL) == BST_CHECKED;
                    s.checkOnStart = IsDlgButtonChecked(h, IDC_ONSTART) == BST_CHECKED;
                    if (s.host.empty() || s.host.find(L' ') != std::wstring::npos) { MessageBoxW(h, L"サーバーのアドレスを入れてください", L"接続の設定", MB_OK | MB_ICONWARNING); return 0; }
                    if (!ValidPort(s.port)) { MessageBoxW(h, L"ポートは 1〜65535 の数字です", L"接続の設定", MB_OK | MB_ICONWARNING); return 0; }
                    if (!s.code.empty() && !ValidCode(s.code)) { MessageBoxW(h, L"接続コードは6桁の数字です", L"接続の設定", MB_OK | MB_ICONWARNING); return 0; }
                    if (s.playerMonitor.empty()) s.playerMonitor = L"0";
                    g_set = s;
                    SaveSettings(g_set);
                    CloseSettings();
                    InvalidateRect(g_main, nullptr, TRUE);
                    return 0;
                }
                case IDC_CANCEL: CloseSettings(); return 0;
            }
            break;
        case WM_CLOSE: CloseSettings(); return 0;
    }
    return DefWindowProcW(h, m, w, l);
}

static void ShowSettings() {
    if (g_setWnd) {
        SetForegroundWindow(g_setWnd);
        return;
    }
    UINT dpi = GetDpiForWindow(g_main);
    RECT r{0, 0, MulDiv(372, dpi, 96), MulDiv(306, dpi, 96)};
    AdjustWindowRectExForDpi(&r, WS_CAPTION | WS_SYSMENU, FALSE, WS_EX_DLGMODALFRAME, dpi);
    RECT pr;
    GetWindowRect(g_main, &pr);
    int w = r.right - r.left, hgt = r.bottom - r.top;
    int x = pr.left + ((pr.right - pr.left) - w) / 2, y = pr.top + ((pr.bottom - pr.top) - hgt) / 2;
    g_setWnd = CreateWindowExW(WS_EX_DLGMODALFRAME, L"JanScopeSettings", L"接続の設定", WS_CAPTION | WS_SYSMENU | WS_VISIBLE, x, y, w, hgt, g_main,
                               nullptr, g_inst, nullptr);
    EnableWindow(g_main, FALSE);
}

// ---- メニュー・ツールバー ----
static HMENU BuildMenu() {
    HMENU bar = CreateMenu();
    HMENU file = CreatePopupMenu();
    AppendMenuW(file, MF_STRING, ID_SETTINGS, L"接続の設定(&S)...");
    AppendMenuW(file, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(file, MF_STRING, ID_OPEN_FOLDER, L"JanScope のフォルダを開く(&F)");
    AppendMenuW(file, MF_STRING, ID_OPEN_DATA, L"対局ログのフォルダを開く(&L)");
    AppendMenuW(file, MF_STRING, ID_OPEN_FFMPEG, L"ffmpeg のフォルダを開く(&M)");
    AppendMenuW(file, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(file, MF_STRING, ID_EXIT, L"終了(&X)");
    AppendMenuW(bar, MF_POPUP, (UINT_PTR)file, L"ファイル(&F)");

    HMENU run = CreatePopupMenu();
    AppendMenuW(run, MF_STRING, ID_SERVER, L"サーバー(&S)");
    AppendMenuW(run, MF_STRING, ID_SERVER_RESUME, L"サーバー（いちばん新しいログから再開）(&R)");
    AppendMenuW(run, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(run, MF_STRING, ID_BROADCAST, L"配信クライアント（出力＋操作）(&B)");
    AppendMenuW(run, MF_STRING, ID_INPUT, L"入力担当(&I)");
    AppendMenuW(run, MF_STRING, ID_PLAYER, L"選手用画面(&P)");
    AppendMenuW(run, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(run, MF_STRING, ID_DEMO, L"画面の確認（サーバーなし）(&D)");
    AppendMenuW(run, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(run, MF_STRING, ID_STOP_ALL, L"起動したものをすべて閉じる(&C)");
    AppendMenuW(bar, MF_POPUP, (UINT_PTR)run, L"起動(&R)");

    HMENU view = CreatePopupMenu();
    AppendMenuW(view, MF_STRING, ID_VIEW_BROADCAST, L"配信クライアントの操作画面(&B)");
    AppendMenuW(view, MF_STRING, ID_VIEW_INPUT, L"入力担当の操作画面(&I)");
    AppendMenuW(view, MF_STRING, ID_VIEW_DEMO, L"画面の確認(&D)");
    AppendMenuW(view, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(view, MF_STRING, ID_VIEW_INFO, L"バージョン情報の表示に戻す(&V)");
    AppendMenuW(bar, MF_POPUP, (UINT_PTR)view, L"表示(&V)");

    HMENU upd = CreatePopupMenu();
    AppendMenuW(upd, MF_STRING, ID_UPDATE_CHECK, L"新しい版を確かめる(&C)");
    AppendMenuW(upd, MF_STRING, ID_UPDATE_APPLY, L"更新する(&U)...");
    AppendMenuW(upd, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(upd, MF_STRING, ID_UPDATE_ON_START, L"起動時に確かめる(&S)");
    AppendMenuW(upd, MF_STRING, ID_RELEASES, L"リリースの一覧を開く(&L)");
    AppendMenuW(bar, MF_POPUP, (UINT_PTR)upd, L"更新(&U)");

    HMENU help = CreatePopupMenu();
    AppendMenuW(help, MF_STRING, ID_README, L"使い方（README）(&R)");
    AppendMenuW(help, MF_STRING, ID_LICENSES, L"ライセンス(&L)");
    AppendMenuW(help, MF_STRING, ID_SOURCE, L"ソースコード（GitHub）(&G)");
    AppendMenuW(help, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(help, MF_STRING, ID_ABOUT, L"バージョン情報(&A)");
    AppendMenuW(bar, MF_POPUP, (UINT_PTR)help, L"ヘルプ(&H)");
    return bar;
}

static void UpdateMenuState() {
    HMENU m = GetMenu(g_main);
    CheckMenuItem(m, ID_UPDATE_ON_START, MF_BYCOMMAND | (g_set.checkOnStart ? MF_CHECKED : MF_UNCHECKED));
    EnableMenuItem(m, ID_UPDATE_APPLY, MF_BYCOMMAND | (g_update.available && !g_updating ? MF_ENABLED : MF_GRAYED));
    EnableMenuItem(m, ID_STOP_ALL, MF_BYCOMMAND | (g_children.empty() ? MF_GRAYED : MF_ENABLED));
    // 起動メニュー: 動いているものに印（選ぶと終了）
    for (UINT fn : kToggles) CheckMenuItem(m, fn, MF_BYCOMMAND | (Running(fn) ? MF_CHECKED : MF_UNCHECKED));
    // 表示メニュー: 埋め込んでいるものだけ選べる。見せているものに印
    const UINT views[][2] = {{ID_VIEW_BROADCAST, ID_BROADCAST}, {ID_VIEW_INPUT, ID_INPUT}, {ID_VIEW_DEMO, ID_DEMO}};
    for (auto& v : views) {
        Child* c = Running(v[1]);
        bool ok = c && c->embedded && IsWindow(c->embedded);
        EnableMenuItem(m, v[0], MF_BYCOMMAND | (ok ? MF_ENABLED : MF_GRAYED));
        CheckMenuItem(m, v[0], MF_BYCOMMAND | (ok && c->embedded == g_embedded ? MF_CHECKED : MF_UNCHECKED));
    }
    CheckMenuItem(m, ID_VIEW_INFO, MF_BYCOMMAND | (g_embedded ? MF_UNCHECKED : MF_CHECKED));
}

struct TbItem {
    UINT id;
    const wchar_t* text;
    const wchar_t* tip;
};
static const TbItem kToolbar[] = {
    {ID_SERVER, L"サーバー", L"処理サーバーを起動します（接続コードは設定のもの）"},
    {ID_BROADCAST, L"配信クライアント", L"出力ウィンドウ（OBS で取り込む）と操作ウィンドウ"},
    {ID_INPUT, L"入力担当", L"入力担当の操作ウィンドウ"},
    {ID_PLAYER, L"選手用画面", L"選手用画面（設定のモニターに出します）"},
    {ID_DEMO, L"画面の確認", L"サーバーなしで見本の画面を出します"},
    {0, nullptr, nullptr},
    {ID_SETTINGS, L"接続の設定", L"サーバーのアドレス・接続コード・選手用画面のモニター"},
    {ID_UPDATE_CHECK, L"更新の確認", L"新しい版があるか確かめます"},
};

/// ツールバー: 文字だけの普通のボタン（BUTTON）を並べる。画像は使わない
static void BuildToolbar(HWND parent) {
    g_tips = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, nullptr, WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX, CW_USEDEFAULT, CW_USEDEFAULT,
                             CW_USEDEFAULT, CW_USEDEFAULT, parent, nullptr, g_inst, nullptr);
    for (const auto& it : kToolbar) {
        if (!it.id) {
            g_buttons.push_back(nullptr);  // 区切り
            continue;
        }
        // 起動するボタンは、動いている間は押された状態（BS_PUSHLIKE のチェックボックス。状態はこちらで付ける）
        bool toggle = std::find(std::begin(kToggles), std::end(kToggles), it.id) != std::end(kToggles);
        DWORD style = toggle ? (BS_CHECKBOX | BS_PUSHLIKE) : BS_PUSHBUTTON;
        HWND b = CreateWindowExW(0, L"BUTTON", it.text, WS_CHILD | WS_VISIBLE | WS_TABSTOP | style, 0, 0, 10, 10, parent,
                                 (HMENU)(INT_PTR)it.id, g_inst, nullptr);
        SendMessageW(b, WM_SETFONT, (WPARAM)g_font, TRUE);
        TOOLINFOW ti{};
        ti.cbSize = sizeof(ti);
        ti.uFlags = TTF_IDISHWND | TTF_SUBCLASS;
        ti.hwnd = parent;
        ti.uId = (UINT_PTR)b;
        ti.lpszText = (LPWSTR)it.tip;
        SendMessageW(g_tips, TTM_ADDTOOLW, 0, (LPARAM)&ti);
        g_buttons.push_back(b);
    }
}

/// ボタンを文字の幅に合わせて左から並べる
static void LayoutToolbar(HWND h) {
    UINT dpi = GetDpiForWindow(h);
    auto s = [dpi](int v) { return MulDiv(v, dpi, 96); };
    HDC dc = GetDC(h);
    HGDIOBJ old = SelectObject(dc, g_font);
    int x = s(6), y = s(5), bh = s(30);
    for (size_t i = 0; i < g_buttons.size(); i++) {
        HWND b = g_buttons[i];
        if (!b) {
            x += s(14);
            continue;
        }
        wchar_t t[64];
        GetWindowTextW(b, t, 64);
        SIZE sz{};
        GetTextExtentPoint32W(dc, t, (int)wcslen(t), &sz);
        int bw = sz.cx + s(24);
        SetWindowPos(b, nullptr, x, y, bw, bh, SWP_NOZORDER | SWP_NOACTIVATE);
        SendMessageW(b, WM_SETFONT, (WPARAM)g_font, TRUE);
        x += bw + s(4);
    }
    SelectObject(dc, old);
    ReleaseDC(h, dc);
    g_barH = y + bh + s(5);
}

static void Layout(HWND h) {
    LayoutToolbar(h);
    SendMessageW(g_status, WM_SIZE, 0, 0);
    RECT rc;
    GetClientRect(h, &rc);
    int parts[2] = {MulDiv(rc.right, 55, 100), -1};
    SendMessageW(g_status, SB_SETPARTS, 2, (LPARAM)parts);
    FitEmbedded();
    InvalidateRect(h, nullptr, TRUE);
}

static void MakeFonts(UINT dpi) {
    if (g_font) DeleteObject(g_font);
    if (g_fontTitle) DeleteObject(g_fontTitle);
    if (g_fontSmall) DeleteObject(g_fontSmall);
    NONCLIENTMETRICSW ncm{};
    ncm.cbSize = sizeof(ncm);
    SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS, sizeof(ncm), &ncm, 0, dpi);
    g_font = CreateFontIndirectW(&ncm.lfMessageFont);
    LOGFONTW t = ncm.lfMessageFont;
    t.lfHeight = -MulDiv(30, dpi, 72);
    t.lfWeight = FW_BOLD;
    g_fontTitle = CreateFontIndirectW(&t);
    LOGFONTW s = ncm.lfMessageFont;
    s.lfHeight = -MulDiv(9, dpi, 72);
    g_fontSmall = CreateFontIndirectW(&s);
}

/// 中央: 名前・版・接続の要約（シンプルに）
static void Paint(HWND h) {
    PAINTSTRUCT ps;
    HDC dc = BeginPaint(h, &ps);
    RECT all;
    GetClientRect(h, &all);
    // ツールバーの帯（ボタンの後ろ）と区切り線
    RECT bar{all.left, all.top, all.right, all.top + g_barH};
    FillRect(dc, &bar, GetSysColorBrush(COLOR_BTNFACE));
    RECT line{all.left, bar.bottom - 1, all.right, bar.bottom};
    FillRect(dc, &line, GetSysColorBrush(COLOR_3DSHADOW));
    RECT rc = ContentRect();
    if (g_embedded) {
        EndPaint(h, &ps);
        return;
    }
    FillRect(dc, &rc, GetSysColorBrush(COLOR_WINDOW));
    SetBkMode(dc, TRANSPARENT);
    UINT dpi = GetDpiForWindow(h);
    auto s = [dpi](int v) { return MulDiv(v, dpi, 96); };

    struct Line {
        std::wstring text;
        HFONT font;
        COLORREF color;
        int gapAfter;
    };
    std::vector<Line> lines;
    lines.push_back({L"JanScope", g_fontTitle, GetSysColor(COLOR_WINDOWTEXT), s(6)});
    lines.push_back({L"バージョン " + g_version, g_font, GetSysColor(COLOR_WINDOWTEXT), s(18)});
    std::wstring conn = L"サーバー " + ServerAddr() + L"　接続コード " + (ValidCode(g_set.code) ? g_set.code : std::wstring(L"（未設定）"));
    lines.push_back({conn, g_fontSmall, GetSysColor(COLOR_GRAYTEXT), s(4)});
    if (g_update.available) {
        lines.push_back({L"新しい版 " + g_update.latest + L" があります（更新 → 更新する）", g_font, RGB(0xB0, 0x60, 0x00), s(4)});
    }
    lines.push_back({L"AI で作成したソフトウェアです。本番で使う前に動作と計算結果を確かめてください", g_fontSmall, GetSysColor(COLOR_GRAYTEXT), 0});

    // 一番上にアイコン（リソースの 1 番）
    int iconPx = s(72);
    HICON icon = (HICON)LoadImageW(g_inst, MAKEINTRESOURCEW(1), IMAGE_ICON, iconPx, iconPx, 0);
    int iconGap = icon ? s(10) : 0;
    int total = icon ? iconPx + iconGap : 0;
    std::vector<int> hts;
    for (auto& ln : lines) {
        SelectObject(dc, ln.font);
        RECT m{0, 0, rc.right, 0};
        DrawTextW(dc, ln.text.c_str(), -1, &m, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
        hts.push_back(m.bottom - m.top);
        total += m.bottom - m.top + ln.gapAfter;
    }
    int y = rc.top + ((rc.bottom - rc.top) - total) / 2;
    if (icon) {
        DrawIconEx(dc, rc.left + ((rc.right - rc.left) - iconPx) / 2, y, icon, iconPx, iconPx, 0, nullptr, DI_NORMAL);
        DestroyIcon(icon);
        y += iconPx + iconGap;
    }
    for (size_t i = 0; i < lines.size(); i++) {
        SelectObject(dc, lines[i].font);
        SetTextColor(dc, lines[i].color);
        RECT r{rc.left, y, rc.right, y + hts[i]};
        DrawTextW(dc, lines[i].text.c_str(), -1, &r, DT_CENTER | DT_SINGLELINE | DT_NOPREFIX);
        y += hts[i] + lines[i].gapAfter;
    }
    EndPaint(h, &ps);
}

static void OnUpdateInfo(UpdateInfo* info, bool manual) {
    g_updateChecking = false;
    g_update = *info;
    delete info;
    if (!g_update.ok || !g_update.error.empty()) {
        SetStatus(1, L"新しい版を確かめられませんでした");
        if (manual) Error(L"新しい版を確かめられませんでした。\n\n" + g_update.error);
    } else if (g_update.available) {
        SetStatus(1, L"新しい版 " + g_update.latest + L" があります");
        if (manual) {
            std::wstring t = L"新しい版 " + g_update.latest + L" があります（今は " + g_update.current + L"）。\n\n";
            if (!g_update.notes.empty()) t += L"変更点:\n" + g_update.notes + L"\n\n";
            t += L"今すぐ更新しますか？";
            if (MessageBoxW(g_main, t.c_str(), L"JanScope の更新", MB_YESNO | MB_ICONINFORMATION) == IDYES) ApplyUpdate();
        }
    } else {
        SetStatus(1, L"最新の版です（" + g_update.current + L"）");
        if (manual) MessageBoxW(g_main, (L"最新の版です（" + g_update.current + L"）").c_str(), L"JanScope の更新", MB_OK | MB_ICONINFORMATION);
    }
    UpdateMenuState();
    InvalidateRect(g_main, nullptr, TRUE);
}

static void OnUpdateDone(DWORD code, std::wstring* out) {
    g_updating = false;
    for (HWND b : g_buttons) EnableWindow(b, TRUE);
    std::wstring o = Trim(*out);
    delete out;
    if (code == 0) {
        SetStatus(1, L"入れ替えました。起動し直すと新しい版になります");
        if (MessageBoxW(g_main, (o + L"\n\nJanScope を起動し直しますか？").c_str(), L"JanScope の更新", MB_YESNO | MB_ICONINFORMATION) == IDYES) {
            STARTUPINFOW si{};
            si.cb = sizeof(si);
            PROCESS_INFORMATION pi{};
            std::wstring cmd = L"\"" + ExePath() + L"\"";
            std::wstring dir = ExeDir();
            if (CreateProcessW(nullptr, cmd.data(), nullptr, nullptr, FALSE, 0, nullptr, dir.c_str(), &si, &pi)) {
                CloseHandle(pi.hThread);
                CloseHandle(pi.hProcess);
                DestroyWindow(g_main);
            }
        }
    } else {
        SetStatus(1, L"入れ替えられませんでした（元のままです）");
        Error(L"入れ替えられませんでした（元のままです）。\n\n" + o);
    }
    UpdateMenuState();
}

static void OnCommand(UINT id) {
    switch (id) {
        case ID_SERVER:
        case ID_BROADCAST:
        case ID_INPUT:
        case ID_PLAYER:
        case ID_DEMO: Toggle(id); break;
        case ID_SERVER_RESUME:
            if (Running(ID_SERVER)) Error(L"サーバーはもう動いています。再開するには、いったん終了してください。");
            else StartServer(true);
            UpdateToggleButtons();
            break;
        case ID_VIEW_BROADCAST: ViewEmbedded(ID_BROADCAST); break;
        case ID_VIEW_INPUT: ViewEmbedded(ID_INPUT); break;
        case ID_VIEW_DEMO: ViewEmbedded(ID_DEMO); break;
        case ID_VIEW_INFO: ShowEmbedded(nullptr); break;
        case ID_STOP_ALL: StopAll(); break;
        case ID_SETTINGS: ShowSettings(); break;
        case ID_EXIT: SendMessageW(g_main, WM_CLOSE, 0, 0); break;
        case ID_UPDATE_CHECK: CheckUpdateAsync(true); break;
        case ID_UPDATE_APPLY: ApplyUpdate(); break;
        case ID_UPDATE_ON_START:
            g_set.checkOnStart = !g_set.checkOnStart;
            SaveSettings(g_set);
            break;
        case ID_RELEASES: Open(kReleasesUrl); break;
        case ID_OPEN_FOLDER: Open(ExeDir()); break;
        case ID_OPEN_DATA: {
            std::wstring d = InDir(L"data");
            CreateDirectoryW(d.c_str(), nullptr);
            Open(d);
            break;
        }
        case ID_OPEN_FFMPEG: {
            std::wstring d = InDir(L"ffmpeg");
            CreateDirectoryW(d.c_str(), nullptr);
            Open(d);
            break;
        }
        case ID_README: {
            std::wstring p = InDir(L"はじめに.txt");
            Open(Exists(p) ? p : InDir(L"README.md"));
            break;
        }
        case ID_LICENSES: Open(InDir(L"THIRD_PARTY_LICENSES.md")); break;
        case ID_SOURCE: Open(kRepoUrl); break;
        case ID_ABOUT: About(); break;
    }
    UpdateMenuState();
}

static LRESULT CALLBACK MainProc(HWND h, UINT m, WPARAM w, LPARAM l) {
    switch (m) {
        case WM_CREATE: {
            g_main = h;
            MakeFonts(GetDpiForWindow(h));
            SetMenu(h, BuildMenu());
            BuildToolbar(h);
            g_status = CreateWindowExW(0, STATUSCLASSNAMEW, nullptr, WS_CHILD | WS_VISIBLE | SBARS_SIZEGRIP, 0, 0, 0, 0, h, nullptr, g_inst, nullptr);
            SendMessageW(g_status, WM_SETFONT, (WPARAM)g_font, TRUE);
            SetTimer(h, TIMER_CHILDREN, 1000, nullptr);
            RefreshChildren();
            UpdateMenuState();
            if (g_set.checkOnStart && Exists(InDir(L"VERSION.txt"))) CheckUpdateAsync(false);
            else SetStatus(1, L"");
            return 0;
        }
        case WM_SIZE: Layout(h); return 0;
        case WM_DPICHANGED: {
            MakeFonts(HIWORD(w));
            SendMessageW(g_status, WM_SETFONT, (WPARAM)g_font, TRUE);
            RECT* r = (RECT*)l;
            SetWindowPos(h, nullptr, r->left, r->top, r->right - r->left, r->bottom - r->top, SWP_NOZORDER | SWP_NOACTIVATE);
            Layout(h);
            return 0;
        }
        case WM_PAINT: Paint(h); return 0;
        case WM_ERASEBKGND: return 1;
        case WM_TIMER:
            if (w == TIMER_CHILDREN) {
                RefreshChildren();
                UpdateMenuState();
            } else if (w == TIMER_EMBED) {
                OnEmbedTimer();
            }
            return 0;
        case WM_SETFOCUS:
            if (g_embedded) SetFocus(g_embedded);
            return 0;
        case WM_PARENTNOTIFY:
            // 埋め込んだ操作ウィンドウが閉じた
            if (LOWORD(w) == WM_DESTROY) RefreshChildren();
            return 0;
        case WM_COMMAND: OnCommand(LOWORD(w)); return 0;
        case WM_APP_UPDATE_INFO: OnUpdateInfo((UpdateInfo*)l, w != 0); return 0;
        case WM_APP_UPDATE_DONE: OnUpdateDone((DWORD)w, (std::wstring*)l); return 0;
        case WM_CLOSE:
            if (!g_children.empty() &&
                MessageBoxW(h, std::any_of(g_children.begin(), g_children.end(), [](const Child& c) { return c.embedded != nullptr; }) ? L"このウィンドウの中の操作画面（と出力ウィンドウ）も閉じます。サーバーなど別のウィンドウのものは動いたままです。\n閉じますか？"
                                          : L"起動したもの（サーバー・クライアントなど）は動いたままになります。\nランチャーだけ閉じますか？",
                            L"JanScope",
                            MB_YESNO | MB_ICONQUESTION) != IDYES)
                return 0;
            DestroyWindow(h);
            return 0;
        case WM_DESTROY: PostQuitMessage(0); return 0;
    }
    return DefWindowProcW(h, m, w, l);
}

int WINAPI wWinMain(HINSTANCE inst, HINSTANCE, LPWSTR, int show) {
    g_inst = inst;
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    INITCOMMONCONTROLSEX icc{sizeof(icc), ICC_BAR_CLASSES | ICC_STANDARD_CLASSES};
    InitCommonControlsEx(&icc);
    g_version = Version();
    g_set = LoadSettings();

    WNDCLASSEXW wc{sizeof(wc)};
    wc.lpfnWndProc = MainProc;
    wc.hInstance = inst;
    wc.hCursor = LoadCursor(nullptr, IDC_ARROW);
    wc.hIcon = LoadIconW(inst, MAKEINTRESOURCEW(1));
    if (!wc.hIcon) wc.hIcon = LoadIcon(nullptr, IDI_APPLICATION);
    wc.hbrBackground = GetSysColorBrush(COLOR_WINDOW);
    wc.lpszClassName = L"JanScopeLauncher";
    RegisterClassExW(&wc);

    WNDCLASSEXW sc{sizeof(sc)};
    sc.lpfnWndProc = SettingsProc;
    sc.hInstance = inst;
    sc.hCursor = LoadCursor(nullptr, IDC_ARROW);
    sc.hbrBackground = GetSysColorBrush(COLOR_BTNFACE);
    sc.lpszClassName = L"JanScopeSettings";
    RegisterClassExW(&sc);

    // 同じものを2つ起動しない（前のを前に出す）
    HANDLE mutex = CreateMutexW(nullptr, TRUE, L"JanScopeLauncherSingleInstance");
    if (GetLastError() == ERROR_ALREADY_EXISTS) {
        HWND other = FindWindowW(L"JanScopeLauncher", nullptr);
        if (other) {
            ShowWindow(other, SW_RESTORE);
            SetForegroundWindow(other);
            return 0;
        }
    }

    UINT dpi = GetDpiForSystem();
    int w = MulDiv(760, dpi, 96), hgt = MulDiv(440, dpi, 96);
    RECT wa;
    SystemParametersInfoW(SPI_GETWORKAREA, 0, &wa, 0);
    HWND h = CreateWindowExW(0, L"JanScopeLauncher", (L"JanScope " + g_version).c_str(), WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                             wa.left + ((wa.right - wa.left) - w) / 2, wa.top + ((wa.bottom - wa.top) - hgt) / 2, w, hgt, nullptr,
                             nullptr, inst, nullptr);
    if (!h) return 1;
    ShowWindow(h, show);
    UpdateWindow(h);

    MSG msg;
    while (GetMessageW(&msg, nullptr, 0, 0) > 0) {
        if (g_setWnd && IsDialogMessageW(g_setWnd, &msg)) continue;
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    if (mutex) CloseHandle(mutex);
    return 0;
}
