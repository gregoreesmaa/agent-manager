// Main window: roster + ConPTY-backed terminal over the core C ABI.
// See MainWindow.xaml.cpp for the epic-DoD wiring table.
#pragma once

#include "MainWindow.g.h"
#include "MainWindow.xaml.g.h"

#include "core_bridge.h"

namespace winrt::AgentManagerWinUI::implementation
{
    struct LivePty
    {
        ::AmPty *pty{nullptr};
        std::string last_snapshot; /* Last full screen text (delta base). */
        std::string shown;         /* Everything fed to the view (capped). */
    };

    struct MainWindow : MainWindowT<MainWindow>
    {
        MainWindow();
        ~MainWindow();

        void NewButton_Click(
            Windows::Foundation::IInspectable const &sender,
            Microsoft::UI::Xaml::RoutedEventArgs const &args);
        void PersistCore();
        void FilterBox_TextChanged(
            Windows::Foundation::IInspectable const &sender,
            Microsoft::UI::Xaml::Controls::TextChangedEventArgs const &args);
        void Roster_SelectionChanged(
            Windows::Foundation::IInspectable const &sender,
            Microsoft::UI::Xaml::Controls::SelectionChangedEventArgs const
                &args);
        void SidebarThumb_DragDelta(
            Windows::Foundation::IInspectable const &sender,
            Microsoft::UI::Xaml::Controls::Primitives::DragDeltaEventArgs const
                &args);
        void SidebarThumb_KeyDown(
            Windows::Foundation::IInspectable const &sender,
            Microsoft::UI::Xaml::Input::KeyRoutedEventArgs const &args);
        void RootGrid_KeyDown(
            Windows::Foundation::IInspectable const &sender,
            Microsoft::UI::Xaml::Input::KeyRoutedEventArgs const &args);

    private:
        void OnTick(
            Windows::Foundation::IInspectable const &sender,
            Windows::Foundation::IInspectable const &args);
        void OnClosed(
            Windows::Foundation::IInspectable const &sender,
            Microsoft::UI::Xaml::WindowEventArgs const &args);
        void RefreshRoster();
        void ShowSelected();
        /* Group-list helpers (issue #73): the four lists share one
         * selection, kept in m_selected; m_syncing guards the
         * SelectionChanged fan-out while the selection is moved. */
        void RebuildGroupList(
            Microsoft::UI::Xaml::Controls::ListView const &list,
            std::vector<std::pair<std::wstring, std::wstring>> const &rows);
        void SelectRowById(std::wstring const &id);
        bool FirstRowId(std::wstring &id);
        /* First roster row with no live PTY yet (display order); false
         * when every visible row is already live or no row is visible. */
        bool FirstUnstartedRowId(std::wstring &id);
        /* Mint a shell-local terminal id ("local-N") for a New Session
         * with no unstarted roster row (empty roster included). Local
         * ids never collide with core roster ids and are always live. */
        std::wstring MintLocalId();
        bool IsLocalId(std::wstring const &id);
        void SetStatus(winrt::hstring const &text);
        void ForwardBytes(char const *data, std::size_t len);
        std::wstring SelectedId();
        LivePty *SelectedLive();
        /* Resizable sidebar: read/apply helpers for the SidebarColumn
         * width behind the Thumb grip. */
        double SidebarWidthPx();
        void SetSidebarWidth(double w);
        fire_and_forget GetContentText(
            Windows::ApplicationModel::DataTransfer::DataPackageView data);

        ::AmCore *m_core{nullptr};
        /* Roster row id -> live PTY, plus shell-local "local-N" terminals
         * (New Session on an empty or fully-live roster). Local entries
         * render in the terminal pane but never in the roster lists. */
        std::map<std::wstring, LivePty> m_live;
        std::wstring m_selected; /* selected roster row id or local id */
        unsigned m_localNext{1}; /* next local terminal number */
        bool m_syncing{false}; /* true while moving shared selection */
        std::string m_filter;                   /* sidebar filter (UTF-8) */
        std::string m_fingerprint; /* roster rebuild gate */
        Microsoft::UI::Dispatching::DispatcherQueueTimer m_timer{nullptr};
        winrt::event_token m_closedToken{};
    };
}

namespace winrt::AgentManagerWinUI::factory_implementation
{
    struct MainWindow : MainWindowT<MainWindow, implementation::MainWindow>
    {
    };
}
