// Application entry point implementation (issue #64).
#include "pch.h"
#include "App.xaml.h"
#include "MainWindow.xaml.h"

using namespace winrt;
using namespace Microsoft::UI::Xaml;

namespace winrt::AgentManagerWinUI::implementation
{
    App::App() {
#if defined _DEBUG && !defined DISABLE_XAML_GENERATED_BREAK_ON_UNHANDLED_EXCEPTION
        UnhandledException(
            [](IInspectable const &, UnhandledExceptionEventArgs const &e) {
                if (IsDebuggerPresent()) {
                    auto message = e.Message();
                    __debugbreak();
                }
            });
#endif
    }

    void App::OnLaunched(LaunchActivatedEventArgs const &) {
        window = make<AgentManagerWinUI::implementation::MainWindow>();
        window.Activate();
    }
}
