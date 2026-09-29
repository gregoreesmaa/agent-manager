// Agent Manager — WinUI 3 shell over the core C ABI (issue #64).
//
// Third native shell after swift/ (#62) and native/linux/ (#63). Every
// run feature goes through `core_bridge.h` (which wraps
// `include/agent_manager.h`): the core owns the roster, the PTYs, and
// the emulator; this shell owns WinUI controls.
//
// Epic DoD wiring (mirrors swift/README.md's table):
//   roster      am_session_count + am_session_json at launch, am_status ticks;
//               live runs grouped needs-input/idle/working, one shared
//               selection across the group lists (#73)
//   spawn       New Session button / Ctrl+N -> bridge_spawn (80x25 grid)
//   converse    key encoding -> bridge_write; pump -> am_feed_delta -> append
//   copy/paste  native TextBox selection + Ctrl+Shift+C; Ctrl+V pastes via
//               Clipboard -> bridge_write; Ctrl+C forwards ETX (interrupts)
//   scroll      output TextBox in a ScrollViewer, per-run text retained
//   search      sidebar search box filters every group; find box selects the
//               next case-insensitive match in the terminal (Ctrl+F focuses)
//   history     collapsed group of rows with no live PTY, restored every
//               launch; per-run output retained while the window lives
//   theme       System/Dark/Light via RequestedTheme, kept in LocalSettings
//   persist     Save button / Ctrl+S / close hook -> bridge_core_save
//
// ConPTY note: no console is ever created on the WinUI side. The core's
// EmbeddedPty on Windows is ConPTY-backed (portable-pty uses the native
// Console Pseudo-terminal API), so bridge_spawn IS the ConPTY spawn; the
// WinUI surface renders core snapshots through feed deltas. Exactly one
// line discipline (the core's) exists, so nothing double-echoes — the
// same single-emulator rule as the Linux shell's "no PTY inside VTE".

#include "pch.h"
#include "MainWindow.xaml.h"
#if __has_include("MainWindow.g.cpp")
#include "MainWindow.g.cpp"
#endif

#include "core_bridge.h"
#include "feed.h"
#include "terminal_keys.h"
#include "json_mini.h"

#include <algorithm>
#include <cctype>

using namespace winrt;
using namespace Microsoft::UI::Xaml;
using namespace Microsoft::UI::Xaml::Controls;
using namespace Microsoft::UI::Xaml::Input;
using namespace Windows::Foundation;

namespace winrt::AgentManagerWinUI::implementation
{
    /* Roster/status refresh rides on the same 50ms pump tick as the
     * Swift and GTK shells. Fixed spawn grid: there is no backing
     * widget grid to measure (the surface is snapshot-fed), so spawns
     * and resizes use the classic 80x25. */
    constexpr int kPumpMs = 50;
    constexpr unsigned kCols = 80;
    constexpr unsigned kRows = 25;
    /* Bounded per-run output (local-only trust + bounded growth: an
     * accumulate-forever buffer would leak memory over long runs). */
    constexpr std::size_t kShownCap = 100000;
    constexpr const char *kFeedClear = AM_FEED_CLEAR;

    static std::wstring to_wide(std::string const &s) {
        if (s.empty()) {
            return {};
        }
        int n = MultiByteToWideChar(
            CP_UTF8, 0, s.c_str(), static_cast<int>(s.size()), nullptr, 0);
        std::wstring w(static_cast<std::size_t>(n), L'\0');
        MultiByteToWideChar(CP_UTF8, 0, s.c_str(),
                            static_cast<int>(s.size()), w.data(), n);
        return w;
    }

    static std::string to_utf8(hstring const &s) {
        auto view = s.c_str();
        int n = WideCharToMultiByte(CP_UTF8, 0, view, -1, nullptr, 0, nullptr,
                                    nullptr);
        if (n <= 1) {
            return {};
        }
        std::string out(static_cast<std::size_t>(n - 1), '\0');
        WideCharToMultiByte(CP_UTF8, 0, view, -1, out.data(), n, nullptr,
                            nullptr);
        return out;
    }

    static hstring status_glyph(int st) {
        switch (st) {
        case AM_STATUS_ATTENTION:
            return L"\u25cf "; /* ● needs input */
        case AM_STATUS_WORKING:
            return L"\u25d0 "; /* ◐ working */
        case AM_STATUS_IDLE:
        default:
            return L"\u25cb "; /* ○ idle */
        }
    }

    /* Resolve a printable UTF-32 code point for a virtual key under the
     * current thread layout (user32; a WinUI 3 desktop app may call it
     * freely). Ctrl+letter bypasses this: with Control held ToUnicode
     * returns control characters, but the encoder wants the letter. */
    static char32_t key_char(int vk, bool ctrl) {
        if (ctrl) {
            if (vk >= 'A' && vk <= 'Z') {
                return static_cast<char32_t>(vk);
            }
            return 0;
        }
        BYTE state[256]{};
        if (!GetKeyboardState(state)) {
            return 0;
        }
        UINT sc = MapVirtualKeyW(static_cast<UINT>(vk), MAPVK_VK_TO_VSC);
        WCHAR buf[8]{};
        int n = ToUnicode(static_cast<UINT>(vk), sc, state, buf, 8, 0);
        if (n == 1) {
            return static_cast<char32_t>(buf[0]);
        }
        /* Dead key or multi-char composition: leave to the control. */
        return 0;
    }

    MainWindow::MainWindow() {
        InitializeComponent();
        /* Native Windows 11 chrome (issue #72): Mica system backdrop,
         * content extended into the title bar with AppTitleBar as the
         * drag region, transparent caption-button wells so Mica shows
         * through. Same controls and handlers: no behavior change. */
        SystemBackdrop(Media::MicaBackdrop{});
        ExtendsContentIntoTitleBar(true);
        SetTitleBar(AppTitleBar());
        try {
            auto titleBar = AppWindow().TitleBar();
            titleBar.ButtonBackgroundColor(
                Microsoft::UI::Colors::Transparent());
            titleBar.ButtonInactiveBackgroundColor(
                Microsoft::UI::Colors::Transparent());
        } catch (...) {
            /* Pre-Windows 11 host: chrome stays default. */
        }
        m_core = bridge_core_new();

        /* Theme: LocalSettings is a plain local store (local-only trust:
         * no account, no sync), mirroring the GTK shell's plain-file
         * pref. Missing key = System, matching first launch. */
        try {
            auto settings =
                Windows::Storage::ApplicationData::Current().LocalSettings();
            auto values = settings.Values();
            if (values.HasKey(L"ThemeIndex")) {
                int idx = unbox_value<int>(values.Lookup(L"ThemeIndex"));
                ThemeBox().SelectedIndex(idx);
                auto theme = ElementTheme::Default;
                if (idx == 1) {
                    theme = ElementTheme::Dark;
                } else if (idx == 2) {
                    theme = ElementTheme::Light;
                }
                RootGrid().RequestedTheme(theme);
            } else {
                ThemeBox().SelectedIndex(0);
            }
        } catch (...) {
            ThemeBox().SelectedIndex(0);
        }

        m_timer = DispatcherQueue().CreateTimer();
        m_timer.Interval(
            std::chrono::milliseconds(kPumpMs));
        m_timer.Tick({this, &MainWindow::OnTick});
        m_timer.Start();

        m_closedToken = Closed({this, &MainWindow::OnClosed});
        RefreshRoster();
        ShowSelected();
        SetStatus(L"Ready.");
    }

    MainWindow::~MainWindow() {
        if (m_timer) {
            m_timer.Stop();
        }
        for (auto &[id, lp] : m_live) {
            (void)id;
            bridge_pty_free(lp.pty);
        }
        m_live.clear();
        bridge_core_free(m_core);
        m_core = nullptr;
    }

    void MainWindow::SetStatus(hstring const &text) {
        StatusText().Text(text);
    }

    std::wstring MainWindow::SelectedId() {
        return m_selected;
    }

    LivePty *MainWindow::SelectedLive() {
        if (m_selected.empty()) {
            return nullptr;
        }
        auto it = m_live.find(m_selected);
        return it == m_live.end() ? nullptr : &it->second;
    }

    void MainWindow::ForwardBytes(char const *data, std::size_t len) {
        LivePty *lp = SelectedLive();
        if (!lp) {
            return;
        }
        char *err = nullptr;
        if (bridge_write(lp->pty, reinterpret_cast<unsigned char const *>(data),
                         len, &err) != 0) {
            std::string msg = "Could not send input: ";
            msg += err ? err : "unknown error";
            SetStatus(to_hstring(msg));
            free(err);
        }
    }

    /* Rebuild the grouped roster only when the fingerprint (row count
     * + filter + per-row status + per-row live-ness) changes; ticks
     * otherwise leave the selection alone. Live runs group by urgency,
     * needs-input first; rows with no live PTY in this shell are
     * history, collapsed at the end. The search box filters every
     * group. Group order is fixed (issue #73): Needs input, Idle,
     * Working, History — every row still shows its status glyph,
     * title, project/harness, restored every launch by am_core_new. */
    void MainWindow::RefreshRoster() {
        if (!m_core) {
            return;
        }
        using Rows = std::vector<std::pair<std::wstring, std::wstring>>;
        size_t n = bridge_session_count(m_core);
        Rows needs;
        Rows idle;
        Rows working;
        Rows history;
        std::string fingerprint;
        fingerprint += std::to_string(n);
        fingerprint += '|';
        fingerprint += m_filter;
        fingerprint += '|';
        for (size_t i = 0; i < n; ++i) {
            char *json = bridge_session_json(m_core, i);
            std::string js = json ? json : "";
            bridge_string_free(json);
            int st = bridge_status(m_core, i);
            std::string id = amjson::get_string(js, "id");
            if (id.empty()) {
                continue;
            }
            std::wstring wid = to_wide(id);
            /* Spawning moves a row from history to a live group without
             * touching its core status, so live-ness joins the gate. */
            bool live = m_live.count(wid) != 0;
            fingerprint += std::to_string(st);
            fingerprint += live ? 'L' : 'h';
            fingerprint += ';';
            if (!m_filter.empty() &&
                js.find(m_filter) == std::string::npos) {
                continue;
            }
            std::string title = amjson::get_string(js, "title");
            std::string project = amjson::get_string(js, "project");
            std::string harness = amjson::get_string(js, "harness");
            std::string line = (title.empty() ? id : title);
            if (!project.empty()) {
                line += " — " + project;
            }
            if (!harness.empty()) {
                line += " · " + harness;
            }
            std::pair<std::wstring, std::wstring> row{
                wid, std::wstring(status_glyph(st)) + to_wide(line)};
            if (!live) {
                history.push_back(std::move(row));
            } else if (st == AM_STATUS_ATTENTION) {
                needs.push_back(std::move(row));
            } else if (st == AM_STATUS_WORKING) {
                working.push_back(std::move(row));
            } else {
                idle.push_back(std::move(row));
            }
        }
        if (fingerprint == m_fingerprint) {
            return;
        }
        m_fingerprint = fingerprint;
        NeedsHeader().Text(winrt::hstring(
            L"Needs input (" + std::to_wstring(needs.size()) + L")"));
        IdleHeader().Text(winrt::hstring(
            L"Idle (" + std::to_wstring(idle.size()) + L")"));
        WorkingHeader().Text(winrt::hstring(
            L"Working (" + std::to_wstring(working.size()) + L")"));
        HistoryExpander().Header(box_value(winrt::hstring(
            L"History (" + std::to_wstring(history.size()) + L")")));
        m_syncing = true;
        RebuildGroupList(NeedsInputList(), needs);
        RebuildGroupList(IdleList(), idle);
        RebuildGroupList(WorkingList(), working);
        RebuildGroupList(HistoryList(), history);
        m_syncing = false;
        if (!m_selected.empty()) {
            SelectRowById(m_selected);
        } else {
            std::wstring first;
            if (FirstRowId(first)) {
                SelectRowById(first);
            }
        }
    }

    /* Repopulate one group list from (id, display) rows. Runs under
     * m_syncing from RefreshRoster, so the clear/repopulate
     * SelectionChanged fan-out is ignored. */
    void MainWindow::RebuildGroupList(
        ListView const &list,
        std::vector<std::pair<std::wstring, std::wstring>> const &rows) {
        list.Items().Clear();
        for (auto const &row : rows) {
            ListViewItem item;
            item.Content(box_value(row.second));
            item.Tag(box_value(row.first));
            list.Items().Append(item);
        }
    }

    /* Move the shared selection to the row with this id, clearing the
     * other three lists. No-op when no list holds the id. */
    void MainWindow::SelectRowById(std::wstring const &id) {
        ListView lists[] = {NeedsInputList(), IdleList(), WorkingList(),
                            HistoryList()};
        bool found = false;
        m_syncing = true;
        for (auto const &list : lists) {
            auto items = list.Items();
            int at = -1;
            for (uint32_t i = 0; i < items.Size(); ++i) {
                auto item = items.GetAt(i).try_as<ListViewItem>();
                if (item &&
                    std::wstring(unbox_value<hstring>(item.Tag())) == id) {
                    at = static_cast<int>(i);
                    found = true;
                    break;
                }
            }
            list.SelectedIndex(at);
        }
        m_syncing = false;
        if (found) {
            m_selected = id;
            ShowSelected();
        }
    }

    /* First row id across the groups in display order; false when every
     * list is empty (no runs yet, or the search matches nothing). */
    bool MainWindow::FirstRowId(std::wstring &id) {
        ListView lists[] = {NeedsInputList(), IdleList(), WorkingList(),
                            HistoryList()};
        for (auto const &list : lists) {
            auto items = list.Items();
            if (items.Size() == 0) {
                continue;
            }
            auto first = items.GetAt(0).try_as<ListViewItem>();
            if (first) {
                id = std::wstring(unbox_value<hstring>(first.Tag()));
                return true;
            }
        }
        return false;
    }

    /* Show the selected run's current output from scratch (selection
     * change or fresh spawn): restore retained per-run text when the
     * run is live, else the latest snapshot, else the empty hint. */
    void MainWindow::ShowSelected() {
        LivePty *lp = SelectedLive();
        if (!lp) {
            TermBox().Text(
                m_selected.empty()
                    ? L"(no run selected)"
                    : L"(no live session — press New Session)");
            return;
        }
        const std::string &text =
            lp->shown.empty() ? lp->last_snapshot : lp->shown;
        TermBox().Text(to_hstring(text));
        auto scroll = TermScroll();
        scroll.UpdateLayout();
        scroll.ChangeView(nullptr, scroll.ScrollableHeight(), nullptr);
    }

    /* The shell's only repaint gate: pump the selected PTY, reconcile
     * the new snapshot against the last one with am_feed_delta, and
     * append (or replay after a clear). */
    void MainWindow::OnTick(IInspectable const &, IInspectable const &) {
        LivePty *lp = SelectedLive();
        if (lp && bridge_pump(lp->pty)) {
            char *snap = bridge_screen_text(lp->pty);
            std::string cur = snap ? snap : "";
            bridge_string_free(snap);
            char *feed = am_feed_delta(lp->last_snapshot.c_str(), cur.c_str());
            if (feed) {
                std::string chunk = feed;
                free(feed);
                if (chunk.compare(0, strlen(kFeedClear), kFeedClear) == 0) {
                    /* Redraw/reflow: clear first, then replay. */
                    lp->shown = chunk.substr(strlen(kFeedClear));
                    TermBox().Text(to_hstring(lp->shown));
                } else {
                    lp->shown += chunk;
                    if (lp->shown.size() > kShownCap) {
                        lp->shown.erase(
                            0, lp->shown.size() - kShownCap);
                    }
                    TermBox().Text(TermBox().Text() + to_hstring(chunk));
                }
                auto scroll = TermScroll();
                scroll.UpdateLayout();
                scroll.ChangeView(nullptr, scroll.ScrollableHeight(),
                                  nullptr);
            }
            lp->last_snapshot = cur;
        }
        RefreshRoster();
    }

    void MainWindow::OnClosed(IInspectable const &,
                              WindowEventArgs const &) {
        if (m_timer) {
            m_timer.Stop();
        }
        if (m_core) {
            /* Close hook persists, mirroring the GTK shell. */
            char *err = nullptr;
            if (bridge_core_save(m_core, &err) != 0) {
                free(err);
            }
        }
    }

    /* New Session starts the selected row's child through the core
     * bridge and selects it in the roster. With no selection yet
     * (fresh launch, or the search cleared it), take the first row
     * in group order so one click always starts something. */
    void MainWindow::NewButton_Click(IInspectable const &,
                                     RoutedEventArgs const &) {
        if (!m_core) {
            return;
        }
        if (m_selected.empty()) {
            std::wstring first;
            if (!FirstRowId(first)) {
                SetStatus(L"No runs yet - nothing to start.");
                return;
            }
            m_selected = first;
        }
        if (m_live.count(m_selected)) {
            SetStatus(L"That run is already live.");
            return;
        }
        char *err = nullptr;
        AmPty *pty = bridge_spawn(m_core, kCols, kRows, &err);
        if (!pty) {
            std::string msg = "Could not spawn: ";
            msg += err ? err : "unknown error";
            SetStatus(to_hstring(msg));
            free(err);
            return;
        }
        LivePty lp;
        lp.pty = pty;
        m_live[m_selected] = std::move(lp);
        RefreshRoster();
        /* The rebuild keeps m_selected; make the lists show the started
         * row (now in a live group) as selected too. */
        SelectRowById(m_selected);
        SetStatus(L"Session started.");
    }

    void MainWindow::SaveButton_Click(IInspectable const &,
                                      RoutedEventArgs const &) {
        if (!m_core) {
            return;
        }
        char *err = nullptr;
        if (bridge_core_save(m_core, &err) != 0) {
            std::string msg = "Could not save: ";
            msg += err ? err : "unknown error";
            SetStatus(to_hstring(msg));
            free(err);
            return;
        }
        SetStatus(L"Saved.");
    }

    void MainWindow::FindNextButton_Click(IInspectable const &,
                                          RoutedEventArgs const &) {
        LivePty *lp = SelectedLive();
        if (!lp) {
            return;
        }
        std::string needle = to_utf8(FindBox().Text());
        if (needle.empty()) {
            return;
        }
        /* Literal, case-insensitive search over the retained text. */
        std::string hay = lp->shown.empty() ? lp->last_snapshot : lp->shown;
        std::string hay_low = hay;
        std::string ndl_low = needle;
        std::transform(hay_low.begin(), hay_low.end(), hay_low.begin(),
                       [](unsigned char c) {
                           return static_cast<char>(std::tolower(c));
                       });
        std::transform(ndl_low.begin(), ndl_low.end(), ndl_low.begin(),
                       [](unsigned char c) {
                           return static_cast<char>(std::tolower(c));
                       });
        std::size_t at = hay_low.find(ndl_low);
        if (at == std::string::npos) {
            SetStatus(L"No match.");
            return;
        }
        /* TextBox indices are UTF-16 code units; snapshots here are
         * byte-compared, so clamp the span into the control text. */
        TermBox().Focus(FocusState::Programmatic);
        int32_t start = static_cast<int32_t>(
            std::min<std::size_t>(at, 1000000000));
        int32_t len = static_cast<int32_t>(
            std::min<std::size_t>(needle.size(), 1000000000));
        if (start + len <= static_cast<int32_t>(TermBox().Text().size())) {
            TermBox().Select(start, len);
        }
        SetStatus(L"Match found.");
    }

    void MainWindow::FilterBox_TextChanged(
        IInspectable const &, TextChangedEventArgs const &) {
        m_filter = to_utf8(FilterBox().Text());
        m_fingerprint.clear(); /* Force a roster rebuild on next tick. */
        RefreshRoster();
    }

    /* One shared selection across the four group lists: a pick in
     * any list clears the other three and shows that run.
     * Null-selection events (list clears during a rebuild) are
     * ignored so m_selected survives the repopulate; m_syncing
     * covers programmatic moves. */
    void MainWindow::Roster_SelectionChanged(
        IInspectable const &sender, SelectionChangedEventArgs const &) {
        if (m_syncing) {
            return;
        }
        auto picked = sender.try_as<ListView>();
        if (!picked) {
            return;
        }
        auto item = picked.SelectedItem().try_as<ListViewItem>();
        if (!item) {
            return;
        }
        m_selected = std::wstring(unbox_value<hstring>(item.Tag()));
        m_syncing = true;
        ListView lists[] = {NeedsInputList(), IdleList(), WorkingList(),
                            HistoryList()};
        for (auto const &list : lists) {
            if (list != picked) {
                list.SelectedIndex(-1);
            }
        }
        m_syncing = false;
        ShowSelected();
    }

    void MainWindow::ThemeBox_SelectionChanged(
        IInspectable const &, SelectionChangedEventArgs const &) {
        int idx = ThemeBox().SelectedIndex();
        auto theme = ElementTheme::Default;
        if (idx == 1) {
            theme = ElementTheme::Dark;
        } else if (idx == 2) {
            theme = ElementTheme::Light;
        }
        RootGrid().RequestedTheme(theme);
        try {
            Windows::Storage::ApplicationData::Current()
                .LocalSettings()
                .Values()
                .Insert(L"ThemeIndex", box_value(idx));
        } catch (...) {
        }
    }

    /* Converse path: every key the encoder accepts becomes child input.
     * App shortcuts (Ctrl+S save, Ctrl+N spawn, Ctrl+F find) ride here
     * too, mirroring the GTK shell's app-level shortcuts. Paste arrives
     * via the clipboard (async); the reserve rule keeps Ctrl+Shift+C/V
     * with the native control for copy. */
    void MainWindow::RootGrid_KeyDown(
        IInspectable const &, KeyRoutedEventArgs const &args) {
        /* WinUI 3 KeyRoutedEventArgs carries no modifiers: query the
         * async key state directly (user32 is free to call here). */
        bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
        bool shift = (GetKeyState(VK_SHIFT) & 0x8000) != 0;
        bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
        int vk = static_cast<int>(args.Key());

        if (ctrl && !alt && !shift) {
            if (vk == 'S') {
                SaveButton_Click(nullptr, nullptr);
                args.Handled(true);
                return;
            }
            if (vk == 'N') {
                NewButton_Click(nullptr, nullptr);
                args.Handled(true);
                return;
            }
            if (vk == 'F') {
                FindBox().Focus(FocusState::Programmatic);
                args.Handled(true);
                return;
            }
            if (vk == 'V') {
                /* Paste: clipboard text becomes child input. */
                auto data =
                    Windows::ApplicationModel::DataTransfer::Clipboard::
                        GetContent();
                if (data.Contains(
                        Windows::ApplicationModel::DataTransfer::
                            StandardDataFormats::Text())) {
                    GetContentText(data);
                }
                args.Handled(true);
                return;
            }
        }

        LivePty *lp = SelectedLive();
        if (!lp) {
            return;
        }
        amkeys::WinKey key;
        key.vk = vk;
        key.text = key_char(vk, ctrl);
        key.ctrl = ctrl;
        key.shift = shift;
        key.alt = alt;
        char buf[8]{};
        std::size_t n = amkeys::encode_key(key, buf);
        if (n == 0) {
            return; /* Native control keeps it (selection, copy, ...). */
        }
        ForwardBytes(buf, n);
        args.Handled(true);
    }

    /* Clipboard read is async; fire-and-forget keeps KeyDown sync. */
    fire_and_forget MainWindow::GetContentText(
        Windows::ApplicationModel::DataTransfer::DataPackageView data) {
        auto lifetime = get_strong();
        try {
            hstring text = co_await data.GetTextAsync();
            std::string utf8 = to_utf8(text);
            /* Pasted newlines go out as CR, like Return. */
            for (char &c : utf8) {
                if (c == '\n') {
                    c = '\r';
                }
            }
            ForwardBytes(utf8.c_str(), utf8.size());
        } catch (...) {
            SetStatus(L"Could not read the clipboard.");
        }
    }
}
