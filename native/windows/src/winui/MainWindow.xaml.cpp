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
//               bridge_spawn_launch (repeat-last: null CLI/folder, zero
//               yolo); picker button / Ctrl+Shift+N -> ContentDialog
//               (folder x CLI + yolo) below
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
#include "picker.h"

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

    /* New Session repeats the last launch instantly through the core
     * bridge (the null-CLI/null-folder/zero-yolo form resolves the
     * effective default, so a picker-confirmed combo repeats here).
     * The fresh PTY mints a shell-local terminal like the empty-roster
     * path already did — one click always opens something and the empty
     * roster is a starting point, not a dead end. A live local stays
     * put (no orphan duplicates): it has no roster row to return to. */
    void MainWindow::NewButton_Click(IInspectable const &,
                                     RoutedEventArgs const &) {
        RepeatLastSession();
    }

    void MainWindow::NewSplitButton_Click(
        IInspectable const &,
        Controls::SplitButtonClickEventArgs const &) {
        RepeatLastSession();
    }

    /* Instant repeat-last shared by the SplitButton face, its menu item,
     * and Ctrl+N: one path, no divergence. */
    void MainWindow::RepeatLastSession() {
        if (!m_core) {
            return;
        }
        /* Live-set cap (same 10-run ceiling as the macOS shell): locals
         * never join the roster, so nothing else would bound them. */
        if (m_live.size() >= 10) {
            SetStatus(L"At 10 live sessions — close one first.");
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
        char *err = nullptr;
        AmPty *pty = bridge_spawn_launch(m_core, nullptr, nullptr, 0,
                                         kCols, kRows, &err);
        if (!pty) {
            std::string msg = "Could not spawn: ";
            msg += err ? err : "unknown error";
            SetStatus(to_hstring(msg));
            free(err);
            return;
        }
        std::wstring id = MintLocalId();
        LivePty lp;
        lp.pty = pty;
        m_live[id] = std::move(lp);
        m_selected = id;
        bridge_note_launch(m_core, nullptr, nullptr, nullptr);
        /* Force the roster rebuild (local PTYs never join its gate):
         * RefreshRoster restores the list selection for a roster row,
         * and clears the list visuals for a local id while keeping
         * m_selected on the new terminal. */
        m_fingerprint.clear();
        RefreshRoster();
        ShowSelected();
        /* Hand the keyboard to the new session (takes the keyboard on
         * spawn, like the macOS shell's `n` key): without this, focus stays on
         * the New Session button, where Return re-clicks instead of
         * submitting the typed prompt. */
        TermBox().Focus(Microsoft::UI::Xaml::FocusState::Programmatic);
        char *eff_raw = bridge_effective_cli(m_core, nullptr);
        std::string eff = eff_raw ? eff_raw : "terminal";
        bridge_string_free(eff_raw);
        SetStatus(hstring{to_wide("New " + eff + " session started.")});
    }

    /* Picker button: open the 2D new-session ContentDialog (folder x
     * CLI + tri-state yolo) over the fresh catalog + recents. */
    void MainWindow::PickButton_Click(IInspectable const &,
                                      RoutedEventArgs const &) {
        PickNewSessionAsync();
    }
    /* 2D new-session dialog. Folder: a TextBox (blank = inherit) plus
     * the persisted recents for one-click refill; a missing folder is
     * reported in the status line with the fix named. CLI: a ComboBox
     * over the autodetected catalog; missing CLIs render disabled with
     * an install hint, never hidden. Yolo: a tri-state ComboBox
     * (Default / On once / Off once), safe by default; the preview line
     * names the exact combination before Spawn. */
    fire_and_forget MainWindow::PickNewSessionAsync() {
        auto lifetime = get_strong();
        if (!m_core) {
            co_return;
        }
        char *clis_raw = bridge_clis_json();
        std::string clis_json = clis_raw ? clis_raw : "[]";
        bridge_string_free(clis_raw);
        char *recents_raw = bridge_recent_json(m_core);
        std::string recents_json = recents_raw ? recents_raw : "[]";
        bridge_string_free(recents_raw);
        auto clis = picker::parse_clis(clis_json);
        auto recents = picker::parse_recents(recents_json);
        if (clis.empty()) {
            SetStatus(L"No agent CLI catalog: cannot open the picker.");
            co_return;
        }

        ComboBox cliBox;
        for (auto const &cli : clis) {
            ComboBoxItem item;
            std::string label = cli.id;
            label += cli.available ? " — ready" : " — not installed";
            if (cli.available && !cli.path.empty()) {
                label += " (" + cli.path + ")";
            }
            item.Content(box_value(to_wide(label)));
            item.Tag(box_value(to_wide(cli.id)));
            item.IsEnabled(cli.available);
            if (!cli.available) {
                ToolTipService::SetToolTip(
                    item, box_value(winrt::hstring(
                              L"Install this CLI and ensure it is on PATH.")));
            }
            cliBox.Items().Append(item);
        }
        /* Preselect the first available CLI (catalog order). */
        for (uint32_t i = 0; i < cliBox.Items().Size(); ++i) {
            auto item = cliBox.Items().GetAt(i).try_as<ComboBoxItem>();
            if (item && item.IsEnabled()) {
                cliBox.SelectedIndex(static_cast<int32_t>(i));
                break;
            }
        }

        TextBox folderBox;
        folderBox.PlaceholderText(L"Blank = current folder");
        folderBox.Text(to_wide(recents.empty() ? "" : recents[0]));

        ComboBox recentBox;
        if (!recents.empty()) {
            recentBox.Items().Append(box_value(winrt::hstring(L"Type a folder…")));
            for (auto const &r : recents) {
                recentBox.Items().Append(box_value(to_wide(r)));
            }
            recentBox.SelectedIndex(0);
        }

        ComboBox yoloBox;
        yoloBox.Items().Append(box_value(winrt::hstring(L"Default")));
        yoloBox.Items().Append(box_value(winrt::hstring(L"On (once)")));
        yoloBox.Items().Append(box_value(winrt::hstring(L"Off (once)")));
        yoloBox.SelectedIndex(0);
        ToolTipService::SetToolTip(
            yoloBox, box_value(winrt::hstring(
                          L"Yolo lets the agent run commands without asking. "
                          L"Default follows the per-agent config; once-choices "
                          L"apply to this run only and are never saved.")));

        TextBlock preview;
        preview.Style(Application::Current()
                          .Resources()
                          .Lookup(box_value(L"CaptionTextBlockStyle"))
                          .as<Style>());
        auto refresh = [&]() {
            int ci = cliBox.SelectedIndex();
            std::string cli =
                (ci >= 0 && static_cast<size_t>(ci) < clis.size())
                    ? clis[static_cast<size_t>(ci)].id
                    : "muse";
            std::string folder = to_utf8(folderBox.Text());
            int yolo = picker::yolo_value(yoloBox.SelectedIndex());
            preview.Text(to_wide(picker::preview(cli, folder, yolo)));
        };
        cliBox.SelectionChanged(
            [&refresh](IInspectable const &, SelectionChangedEventArgs const &) {
                refresh();
            });
        yoloBox.SelectionChanged(
            [&refresh](IInspectable const &, SelectionChangedEventArgs const &) {
                refresh();
            });
        folderBox.TextChanged(
            [&](IInspectable const &, TextChangedEventArgs const &) {
                refresh();
            });
        if (!recents.empty()) {
            recentBox.SelectionChanged(
                [&](IInspectable const &,
                    SelectionChangedEventArgs const &) {
                    int ri = recentBox.SelectedIndex();
                    if (ri > 0 &&
                        static_cast<size_t>(ri - 1) < recents.size()) {
                        folderBox.Text(to_wide(recents[static_cast<size_t>(ri - 1)]));
                    }
                    refresh();
                });
        }
        refresh();

        StackPanel panel;
        panel.Spacing(8);
        auto head = [](const wchar_t *t) {
            TextBlock h;
            h.Text(t);
            h.Style(Application::Current()
                        .Resources()
                        .Lookup(box_value(L"SubtitleTextBlockStyle"))
                        .as<Style>());
            return h;
        };
        panel.Children().Append(head(L"Where should it work?"));
        panel.Children().Append(folderBox);
        if (!recents.empty()) {
            panel.Children().Append(recentBox);
        }
        panel.Children().Append(head(L"Who should do it?"));
        panel.Children().Append(cliBox);
        panel.Children().Append(head(L"Permission mode"));
        panel.Children().Append(yoloBox);
        panel.Children().Append(preview);

        ContentDialog dialog;
        dialog.Title(box_value(winrt::hstring(L"Start a new run")));
        dialog.Content(panel);
        dialog.PrimaryButtonText(L"Spawn");
        dialog.CloseButtonText(L"Cancel");
        dialog.DefaultButton(ContentDialogButton::Primary);
        dialog.XamlRoot(this->Content().XamlRoot());
        /* Spawn stays disabled while a missing CLI is selected: the
         * failure would be certain, so prevent it inline (fail visible
         * at the control, not after the click). */
        auto sync_spawn = [&]() {
            int ci = cliBox.SelectedIndex();
            bool avail =
                (ci >= 0 && static_cast<size_t>(ci) < clis.size()) &&
                clis[static_cast<size_t>(ci)].available;
            dialog.IsPrimaryButtonEnabled(avail);
        };
        cliBox.SelectionChanged(
            [&sync_spawn](IInspectable const &, SelectionChangedEventArgs const &) {
                sync_spawn();
            });
        sync_spawn();
        auto result = co_await dialog.ShowAsync();
        if (result != ContentDialogResult::Primary) {
            co_return;
        }
        int ci = cliBox.SelectedIndex();
        std::string cli =
            (ci >= 0 && static_cast<size_t>(ci) < clis.size())
                ? clis[static_cast<size_t>(ci)].id
                : "muse";
        std::string folder = picker::effective_folder(to_utf8(folderBox.Text()));
        /* is-dir check inline (fail visible, dialog already closed by
         * ShowAsync: report in the status line with the fix named). */
        if (!folder.empty()) {
            DWORD attrs = GetFileAttributesA(folder.c_str());
            if (attrs == INVALID_FILE_ATTRIBUTES ||
                !(attrs & FILE_ATTRIBUTE_DIRECTORY)) {
                SetStatus(winrt::hstring(to_wide("No such folder: " + folder +
                                                 " — reopen the picker to fix it.")));
                co_return;
            }
        }
        int yolo = picker::yolo_value(yoloBox.SelectedIndex());
        char *err = nullptr;
        AmPty *pty = bridge_spawn_launch(
            m_core, cli.c_str(), folder.empty() ? nullptr : folder.c_str(),
            yolo, kCols, kRows, &err);
        if (!pty) {
            std::string msg = "Could not spawn: ";
            msg += err ? err : "unknown error";
            SetStatus(to_hstring(msg));
            free(err);
            co_return;
        }
        std::wstring id = MintLocalId();
        LivePty lp;
        lp.pty = pty;
        m_live[id] = std::move(lp);
        m_selected = id;
        bridge_note_launch(m_core, cli.c_str(),
                           folder.empty() ? nullptr : folder.c_str(),
                           nullptr);
        m_fingerprint.clear();
        RefreshRoster();
        ShowSelected();
        SetStatus(winrt::hstring(to_wide(picker::preview(cli, folder, yolo))));
        /* Hand the keyboard to the new session (takes the keyboard on
         * spawn, like the macOS shell's `n` key): without this, focus stays on
         * the New Session button, where Return re-clicks instead of
         * submitting the typed prompt. */
        TermBox().Focus(Microsoft::UI::Xaml::FocusState::Programmatic);
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
     * App shortcuts (Ctrl+S persist, Ctrl+N repeat-last spawn,
     * Ctrl+Shift+N picker) ride here too, mirroring the GTK shell's
     * app-level shortcuts. Paste arrives via the clipboard (async);
     * the reserve rule keeps Ctrl+Shift+C/V with the native control
     * for copy.
     *
     * Two documented converse keys never reach this bubbling handler:
     * the read-only output box swallows Return (newline insertion) and
     * plain Ctrl+C (copy) before they bubble. Those ride
     * `TermBox_PreviewKeyDown` (tunneling) instead, through the same
     * encoder below — one key table, no fork. */
    void MainWindow::RootGrid_KeyDown(
        IInspectable const &, KeyRoutedEventArgs const &args) {
        /* WinUI 3 KeyRoutedEventArgs carries no modifiers: query the
         * async key state directly (user32 is free to call here). */
        bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
        bool shift = (GetKeyState(VK_SHIFT) & 0x8000) != 0;
        bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
        int vk = static_cast<int>(args.Key());

        if (ctrl && !alt) {
            if (!shift && vk == 'S') {
                PersistCore();
                args.Handled(true);
                return;
            }
            if (!shift && vk == 'N') {
                /* Ctrl+N repeats the last launch instantly. */
                NewButton_Click(nullptr, nullptr);
                args.Handled(true);
                return;
            }
            if (shift && vk == 'N') {
                /* Ctrl+Shift+N opens the full picker. */
                PickButton_Click(nullptr, nullptr);
                args.Handled(true);
                return;
            }
            if (!shift && vk == 'V') {
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

    /* Tunneling converse keys for the terminal surface: the read-only
     * output box swallows Return (newline insertion) and plain Ctrl+C
     * (copy) before they can bubble to `RootGrid_KeyDown`, so without
     * this handler a prompt can be typed but never submitted and the
     * child can never be interrupted. Only these two documented keys
     * are intercepted here — everything else flows untouched, so text
     * selection, roster navigation, and the search box keep working.
     * Encoding reuses `amkeys::encode_key` (the same table the bubble
     * handler uses); the reserve rule stays intact because
     * Ctrl+Shift+C/V encode to nothing and fall through to the box. */
    void MainWindow::TermBox_PreviewKeyDown(
        IInspectable const &, KeyRoutedEventArgs const &args) {
        int vk = static_cast<int>(args.Key());
        bool isReturn = (vk == amkeys::kVkReturn);
        bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
        bool shift = (GetKeyState(VK_SHIFT) & 0x8000) != 0;
        bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
        bool isPlainCtrlC = ctrl && !shift && !alt && (vk == 'C');
        if (!isReturn && !isPlainCtrlC) {
            return;
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
            return; /* Reserved for the control (e.g. Ctrl+Shift+C). */
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
