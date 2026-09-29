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
        void SaveButton_Click(
            Windows::Foundation::IInspectable const &sender,
            Microsoft::UI::Xaml::RoutedEventArgs const &args);
        void FindNextButton_Click(
            Windows::Foundation::IInspectable const &sender,
            Microsoft::UI::Xaml::RoutedEventArgs const &args);
        void FilterBox_TextChanged(
            Windows::Foundation::IInspectable const &sender,
            Microsoft::UI::Xaml::Controls::TextChangedEventArgs const &args);
        void RosterList_SelectionChanged(
            Windows::Foundation::IInspectable const &sender,
            Microsoft::UI::Xaml::Controls::SelectionChangedEventArgs const
                &args);
        void ThemeBox_SelectionChanged(
            Windows::Foundation::IInspectable const &sender,
            Microsoft::UI::Xaml::Controls::SelectionChangedEventArgs const
                &args);
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
        void SetStatus(winrt::hstring const &text);
        void ForwardBytes(char const *data, std::size_t len);
        std::wstring SelectedId();
        LivePty *SelectedLive();
        fire_and_forget GetContentText(
            Windows::ApplicationModel::DataTransfer::DataPackageView data);

        ::AmCore *m_core{nullptr};
        std::map<std::wstring, LivePty> m_live; /* roster id -> live PTY */
        std::wstring m_selected;                /* selected roster row id */
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
