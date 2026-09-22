using System;
using System.Drawing;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using Microsoft.UI.Dispatching;
using Forms = System.Windows.Forms;

namespace Porthole.WindowsHelper;

// The status window is WinUI; NotifyIcon supplies only the native shell entry.
internal sealed class TrayShell : IDisposable
{
    [DllImport("user32.dll", SetLastError = true)]
    static extern bool DestroyIcon(IntPtr icon);

    readonly Forms.NotifyIcon icon;
    readonly Forms.ContextMenuStrip menu;
    readonly Forms.ToolStripMenuItem quit;
    readonly Icon artwork;

    public TrayShell(DispatcherQueue dispatcher, Action open, Action exit)
    {
        artwork = LoadArtwork();
        menu = new Forms.ContextMenuStrip();
        menu.Items.Add("Open Porthole", null, (_, _) => dispatcher.TryEnqueue(() => open()));
        menu.Items.Add(new Forms.ToolStripSeparator());
        quit = new Forms.ToolStripMenuItem("Quit helper", null, (_, _) => dispatcher.TryEnqueue(() => exit()));
        menu.Items.Add(quit);
        icon = new Forms.NotifyIcon {
            Icon = artwork,
            Text = "Porthole helper",
            ContextMenuStrip = menu,
            Visible = true,
        };
        icon.MouseClick += (_, e) => {
            if (e.Button == Forms.MouseButtons.Left) dispatcher.TryEnqueue(() => open());
        };
    }

    public void SetBusy(bool busy) => quit.Enabled = !busy;

    static Icon LoadArtwork()
    {
        using Stream stream = Assembly.GetExecutingAssembly().GetManifestResourceStream("PortholeIcon.png")
            ?? throw new InvalidOperationException("Porthole tray artwork is missing");
        using var source = new Bitmap(stream);
        using var scaled = new Bitmap(source, new Size(32, 32));
        IntPtr handle = scaled.GetHicon();
        try { return (Icon)Icon.FromHandle(handle).Clone(); }
        finally { DestroyIcon(handle); }
    }

    public void Dispose()
    {
        icon.Visible = false;
        icon.Dispose();
        menu.Dispose();
        artwork.Dispose();
    }
}
