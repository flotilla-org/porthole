using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Input;
using Windows.System;

namespace Porthole.WindowsHelper;

public sealed partial class HandoffWindow : Window
{
    public HandoffWindow() { InitializeComponent(); }

    void OnFlyoutKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key != VirtualKey.Escape) return;
        e.Handled = true;
        DismissRequested?.Invoke();
    }

    internal event System.Action? DismissRequested;
}
