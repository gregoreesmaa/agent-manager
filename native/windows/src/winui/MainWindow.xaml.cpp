// Agent Manager — WinUI 3 shell over the core C ABI (issue #64).
//
// Third native shell after swift/ (#62) and native/linux/ (#63). Every
// run feature goes through `core_bridge.h` (which wraps
// `include/agent_manager.h`): the core owns the roster, the PTYs, and
// the emulator; this shell owns WinUI controls.
//
// Epic DoD wiring (mirrors swift/README.md's table):
//   roster      am_session_count + am_session_json at launch, am_status ticks;
//               live runs grouped needs-input/working/idle (macOS parity),
//               one shared selection across the group lists (#73); rows show
//               age + link badges from the core (am_last_active /
//               am_relative_age / am_link_count), filtered by
//               am_roster_matches; no heading, no idle status line
//   spawn       New Session button / Ctrl+N / empty-overlay button ->
//               bridge_spawn (80x24 grid)
//   converse    key encoding -> bridge_write; pump -> bridge_feed_delta
//               (core reconciler) -> append
//   copy/paste  native TextBox selection + Ctrl+Shift+C; Ctrl+V pastes via
//               Clipboard -> bridge_write; Ctrl+C forwards ETX (interrupts)
//   scroll      output TextBox in a ScrollViewer, per-run text retained
//   search      sidebar search box filters every group
//   history     collapsed group of rows with no live PTY, restored every
//               launch; per-run output retained while the window lives
//   persist     Ctrl+S / close hook -> bridge_core_save (no sidebar control)
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
#include "terminal_keys.h"
#include "json_mini.h"

#include <ctime>
#include <utility>

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
     * use the shared 80x24 default the macOS shell starts from. */
    constexpr int kPumpMs = 50;
    constexpr unsigned kCols = 80;
    constexpr unsigned kRows = 24;
    /* Bounded per-run output (local-only trust + bounded growth: an
     * accumulate-forever buffer would leak memory over long runs). */
    constexpr std::size_t kShownCap = 100000;
    /* Redraw marker prefixing core clear-and-replay feeds: must match
     * the core's FEED_CLEAR (`shell_shared`), checked after every
     * `bridge_feed_delta` below. */
    constexpr const char *kFeedClear = "\x1b[2J\x1b[H";
    /* Resizable sidebar: the Thumb between the roster card and the
     * terminal card drives SidebarColumn (the XAML default is 320px;
     * clamped to the GTK shell's 220px floor and a 480px ceiling so
     * long titles stay glanceable). The width persists in
     * LocalSettings (local-only trust: plain local store, no
     * account, no sync). */
    constexpr double kSidebarMin = 220.0;
    constexpr double kSidebarMax = 480.0;
    constexpr double kSidebarKeyStep = 8.0;
    constexpr wchar_t const *kSidebarWidthKey = L"SidebarWidth";

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

        /* Restore the persisted sidebar width (LocalSettings is a
         * plain local store — local-only trust: no account, no
         * sync). Out-of-range values fall back to the 320px XAML
         * default. */
        try {
            auto values = Windows::Storage::ApplicationData::Current()
                              .LocalSettings()
                              .Values();
            if (values.HasKey(kSidebarWidthKey)) {
                double w =
                    unbox_value<double>(values.Lookup(kSidebarWidthKey));
                if (w >= kSidebarMin && w <= kSidebarMax) {
                    SidebarColumn().Width(GridLengthHelper::FromPixels(w));
                }
            }
        } catch (...) {
        }

        m_timer = DispatcherQueue().CreateTimer();
        m_timer.Interval(
            std::chrono::milliseconds(kPumpMs));
        m_timer.Tick({this, &MainWindow::OnTick});
        m_timer.Start();

        m_closedToken = Closed({this, &MainWindow::OnClosed});
        RefreshRoster();
        ShowSelected();
        /* No "Ready." banner: the status line stays empty until a real
         * failure needs it (fail-visible; macOS shows no status either). */
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
     * group through the core match (case-insensitive title/project/id,
     * like the macOS sidebar). Group order is fixed: Needs input,
     * Working, Idle, History (macOS parity) — every row still shows
     * its status glyph, title, project/harness, relative age, and link
     * badge, restored every launch by am_core_new. */
    void MainWindow::RefreshRoster() {
        if (!m_core) {
            return;
        }
        using Rows = std::vector<std::pair<std::wstring, std::wstring>>;
        size_t n = bridge_session_count(m_core);
        Rows needs;
        Rows working;
        Rows idle;
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
            std::string title = amjson::get_string(js, "title");
            std::string project = amjson::get_string(js, "project");
            std::string harness = amjson::get_string(js, "harness");
            if (!bridge_roster_matches(title.c_str(), project.c_str(),
                                       id.c_str(), m_filter.c_str())) {
                continue;
            }
            std::string line = (title.empty() ? id : title);
            if (!project.empty()) {
                line += " — " + project;
            }
            if (!harness.empty()) {
                line += " · " + harness;
            }
            long long last = bridge_last_active(m_core, i);
            if (last >= 0) {
                char *age =
                    bridge_relative_age((long long)std::time(nullptr), last);
                if (age) {
                    line += " · ";
                    line += age;
                    bridge_string_free(age);
                }
            }
            int links = bridge_link_count(m_core, i);
            if (links > 0) {
                line += " · " + std::to_string(links) +
                        (links == 1 ? " link" : " links");
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
        WorkingHeader().Text(winrt::hstring(
            L"Working (" + std::to_wstring(working.size()) + L")"));
        IdleHeader().Text(winrt::hstring(
            L"Idle (" + std::to_wstring(idle.size()) + L")"));
        HistoryExpander().Header(box_value(winrt::hstring(
            L"History (" + std::to_wstring(history.size()) + L")")));
        m_syncing = true;
        RebuildGroupList(NeedsInputList(), needs);
        RebuildGroupList(WorkingList(), working);
        RebuildGroupList(IdleList(), idle);
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
        ListView lists[] = {NeedsInputList(), WorkingList(), IdleList(),
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
        ListView lists[] = {NeedsInputList(), WorkingList(), IdleList(),
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

    bool MainWindow::FirstUnstartedRowId(std::wstring &id) {
        ListView lists[] = {NeedsInputList(), WorkingList(), IdleList(),
                            HistoryList()};
        for (auto const &list : lists) {
            auto items = list.Items();
            for (uint32_t i = 0; i < items.Size(); ++i) {
                auto item = items.GetAt(i).try_as<ListViewItem>();
                if (!item) {
                    continue;
                }
                std::wstring row(unbox_value<hstring>(item.Tag()));
                if (!m_live.count(row)) {
                    id = row;
                    return true;
                }
            }
        }
        return false;
    }

    bool MainWindow::IsLocalId(std::wstring const &id) {
        constexpr wchar_t kPrefix[] = L"local-";
        return id.compare(0, 6, kPrefix) == 0;
    }

    std::wstring MainWindow::MintLocalId() {
        for (;;) {
            std::wstring id = L"local-" + std::to_wstring(m_localNext++);
            if (!m_live.count(id)) {
                return id;
            }
        }
    }

    /* Index of the roster row with this id, or -1 (local terminal ids
     * and stale selections have no roster row). */
    long long MainWindow::RowIndexById(std::wstring const &id) {
        if (!m_core || id.empty()) {
            return -1;
        }
        std::string want = to_utf8(hstring(id));
        size_t n = bridge_session_count(m_core);
        for (size_t i = 0; i < n; ++i) {
            char *json = bridge_session_json(m_core, i);
            std::string js = json ? json : "";
            bridge_string_free(json);
            if (amjson::get_string(js, "id") == want) {
                return static_cast<long long>(i);
            }
        }
        return -1;
    }

    /* Show one empty-overlay state (macOS parity): title + detail +
     * one prominent button, or no button when there is nothing to
     * start. The terminal surface hides behind the overlay. */
    void MainWindow::ShowEmpty(std::wstring const &title,
                               std::wstring const &detail,
                               std::wstring const &button) {
        TermScroll().Visibility(Visibility::Collapsed);
        EmptyTitle().Text(hstring(title));
        EmptyDetail().Text(hstring(detail));
        if (button.empty()) {
            EmptyButton().Visibility(Visibility::Collapsed);
        } else {
            EmptyButton().Content(box_value(hstring(button)));
            EmptyButton().Visibility(Visibility::Visible);
        }
        EmptyPanel().Visibility(Visibility::Visible);
    }

    /* Show the selected run: the live terminal surface when its PTY is
     * live, else the empty overlay — the selected row's title with a
     * Spawn button, "Select a session" when nothing is picked, or the
     * "No sessions yet" CTA on an empty roster (all macOS parity). */
    void MainWindow::ShowSelected() {
        LivePty *lp = SelectedLive();
        if (lp) {
            EmptyPanel().Visibility(Visibility::Collapsed);
            TermScroll().Visibility(Visibility::Visible);
            const std::string &text =
                lp->shown.empty() ? lp->last_snapshot : lp->shown;
            TermBox().Text(to_hstring(text));
            auto scroll = TermScroll();
            scroll.UpdateLayout();
            scroll.ChangeView(nullptr, scroll.ScrollableHeight(), nullptr);
            return;
        }
        long long at = RowIndexById(m_selected);
        if (at >= 0) {
            char *json = bridge_session_json(
                m_core, static_cast<size_t>(at));
            std::string js = json ? json : "";
            bridge_string_free(json);
            std::string title = amjson::get_string(js, "title");
            std::string project = amjson::get_string(js, "project");
            std::string harness = amjson::get_string(js, "harness");
            if (title.empty()) {
                title = amjson::get_string(js, "id");
            }
            std::string detail = project;
            if (!harness.empty()) {
                if (!detail.empty()) {
                    detail += " · ";
                }
                detail += harness;
            }
            ShowEmpty(to_wide(title), to_wide(detail), L"Spawn session");
            return;
        }
        if (bridge_session_count(m_core) == 0) {
            ShowEmpty(L"No sessions yet",
                      L"Spawned sessions appear here; history is restored on "
                      L"launch.",
                      L"New Session");
            return;
        }
        ShowEmpty(L"Select a session", L"", L"");
    }

    /* The shell's only repaint gate: pump the selected PTY, reconcile
     * the new snapshot against the last one with the core feed
     * reconciler, and append (or replay after a clear). */
    void MainWindow::OnTick(IInspectable const &, IInspectable const &) {
        LivePty *lp = SelectedLive();
        if (lp && bridge_pump(lp->pty)) {
            char *snap = bridge_screen_text(lp->pty);
            std::string cur = snap ? snap : "";
            bridge_string_free(snap);
            char *feed =
                bridge_feed_delta(lp->last_snapshot.c_str(), cur.c_str());
            if (feed) {
                std::string chunk = feed;
                bridge_string_free(feed);
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

    /* New Session opens a live CLI terminal through the core bridge.
     * The selected roster row starts when it has no live PTY yet (same
     * as before); with no unstarted row (empty roster included) or an
     * already-live roster selection, it mints a shell-local terminal
     * instead — so one click always opens something and the empty
     * roster is a starting point, not a dead end. A live local stays
     * put (no orphan duplicates): it has no roster row to return to. */
    void MainWindow::NewButton_Click(IInspectable const &,
                                     RoutedEventArgs const &) {
        if (!m_core) {
            return;
        }
        /* A live local terminal is already the newest thing open:
         * keep it selected rather than orphaning it behind a newer
         * one (locals have no roster row to navigate back to). */
        if (!m_selected.empty() && IsLocalId(m_selected) &&
            m_live.count(m_selected) != 0) {
            SetStatus(L"Terminal already open.");
            return;
        }
        /* Unstarted roster selection starts as before; an already-live
         * roster selection or an empty/exhausted roster mints a
         * shell-local terminal instead, so one click always opens
         * something. */
        std::wstring id;
        bool local = false;
        if (!m_selected.empty() && !IsLocalId(m_selected) &&
            m_live.count(m_selected) != 0) {
            id = MintLocalId();
            local = true;
        } else if (m_selected.empty() || IsLocalId(m_selected)) {
            if (!FirstUnstartedRowId(id)) {
                id = MintLocalId();
                local = true;
            }
        } else {
            id = m_selected;
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
        m_live[id] = std::move(lp);
        m_selected = id;
        /* Force the roster rebuild (local PTYs never join its gate):
         * RefreshRoster restores the list selection for a roster row,
         * and clears the list visuals for a local id while keeping
         * m_selected on the new terminal. */
        m_fingerprint.clear();
        RefreshRoster();
        ShowSelected();
        SetStatus(local ? L"New terminal started." : L"Session started.");
    }

    void MainWindow::PersistCore() {
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
        ListView lists[] = {NeedsInputList(), WorkingList(), IdleList(),
                            HistoryList()};
        for (auto const &list : lists) {
            if (list != picked) {
                list.SelectedIndex(-1);
            }
        }
        m_syncing = false;
        ShowSelected();
    }

    /* Current sidebar width in pixels; 0 when the column is star/auto
     * (never expected — the XAML pins pixels — but guarded anyway). */
    double MainWindow::SidebarWidthPx() {
        return SidebarColumn().ActualWidth();
    }

    /* Clamp + apply + persist one sidebar width. Persistence rides
     * LocalSettings (local-only trust: plain local store). */
    void MainWindow::SetSidebarWidth(double w) {
        double clamped = std::clamp(w, kSidebarMin, kSidebarMax);
        SidebarColumn().Width(GridLengthHelper::FromPixels(clamped));
        try {
            Windows::Storage::ApplicationData::Current()
                .LocalSettings()
                .Values()
                .Insert(kSidebarWidthKey, box_value(clamped));
        } catch (...) {
        }
    }

    /* Thumb drag: HorizontalChange is already in DIPs along the drag
     * axis, so it adds straight onto the column width. (Unlike
     * KeyRoutedEventArgs, DragDeltaEventArgs carries no Handled flag
     * — there is nothing to mark: the event has no routing to stop.) */
    void MainWindow::SidebarThumb_DragDelta(
        IInspectable const &,
        Controls::Primitives::DragDeltaEventArgs const &args) {
        SetSidebarWidth(SidebarWidthPx() + args.HorizontalChange());
    }

    /* Keyboard parity for the grip (fail-visible + non-color cue: the
     * Thumb template's grip lights up on focus/hover/press, and the
     * thumb exposes an automation name + tooltip): Left/Right nudge
     * in 8px steps, Home/End jump to min/max. Up/Down mirror
     * Left/Right for screen-reader arrow conventions; anything else
     * stays with the shell's global KeyDown handler. */
    void MainWindow::SidebarThumb_KeyDown(
        IInspectable const &, KeyRoutedEventArgs const &args) {
        double w = SidebarWidthPx();
        switch (args.Key()) {
        case Windows::System::VirtualKey::Left:
        case Windows::System::VirtualKey::Up:
            SetSidebarWidth(w - kSidebarKeyStep);
            args.Handled(true);
            return;
        case Windows::System::VirtualKey::Right:
        case Windows::System::VirtualKey::Down:
            SetSidebarWidth(w + kSidebarKeyStep);
            args.Handled(true);
            return;
        case Windows::System::VirtualKey::Home:
            SetSidebarWidth(kSidebarMin);
            args.Handled(true);
            return;
        case Windows::System::VirtualKey::End:
            SetSidebarWidth(kSidebarMax);
            args.Handled(true);
            return;
        default:
            return;
        }
    }

    /* Converse path: every key the encoder accepts becomes child input.
     * App shortcuts (Ctrl+S persist, Ctrl+N spawn) ride here too,
     * mirroring the GTK shell's app-level shortcuts. Paste arrives
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
                PersistCore();
                args.Handled(true);
                return;
            }
            if (vk == 'N') {
                NewButton_Click(nullptr, nullptr);
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
