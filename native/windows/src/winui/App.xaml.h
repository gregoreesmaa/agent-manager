// Application entry point (issue #64). Standard unpackaged WinUI 3
// boilerplate: create the main window on launch and keep it alive.
#pragma once

#include "App.g.h"
#include "App.xaml.g.h"

namespace winrt::StaapWinUI::implementation
{
    struct App : AppT<App>
    {
        App();

        void OnLaunched(
            Microsoft::UI::Xaml::LaunchActivatedEventArgs const &);

    private:
        Microsoft::UI::Xaml::Window window{nullptr};
    };
}

namespace winrt::StaapWinUI::factory_implementation
{
    struct App : AppT<App, implementation::App>
    {
    };
}
