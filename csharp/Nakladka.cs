// Indeks - silnik nakladki (C#).
// Gdy aplikacja na wierzchu zajmuje caly monitor (gra, film) albo jest zmaksymalizowana,
// pokazuje w lewym dolnym rogu polprzezroczyste CPU / GPU / RAM (i osobno dyski).
// Okno warstwowe: przezroczyste dla klikniec (klik trafia do gry), nigdy sie nie aktywuje,
// co 0,5 s wraca na wierzch warstwy "zawsze na wierzchu" (gry bez ramki tez sa TOPMOST).
// Ustawienia czyta z %APPDATA%\Indeks\indeks.cfg, konczy sie razem z Indeksem (--rodzic PID).
// Kompilacja: csc z .NET Framework 4 (jest w kazdym Windows 10/11) - patrz build.rs.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Text;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

static class Program
{
    [STAThread]
    static int Main(string[] args)
    {
        // --zrzut plik.png: sama nakladka do pliku (podglad wygladu bez gry na pelnym ekranie)
        if (args.Length >= 2 && args[0] == "--zrzut")
        {
            N.SetProcessDPIAware();
            var st = new Stats();
            st.Update();
            Thread.Sleep(1000);
            st.Update();
            var c = new Cfg();
            c.Reload();
            using (Bitmap b = Nakladka.Render(st, c, 1.5f)) b.Save(args[1]);
            return 0;
        }
        bool created;
        using (new Mutex(true, @"Local\Indeks.Nakladka", out created))
        {
            if (!created) return 0;
            try { N.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { N.SetProcessDPIAware(); }
            int parent = 0;
            for (int i = 0; i + 1 < args.Length; i++)
                if (args[i] == "--rodzic") int.TryParse(args[i + 1], out parent);
            // wyjatek nie zamyka silnika - trafia do dziennika
            Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);
            Application.ThreadException += delegate(object s, ThreadExceptionEventArgs e) { Nakladka.Log("wyjatek: " + e.Exception); };
            AppDomain.CurrentDomain.UnhandledException += delegate(object s, UnhandledExceptionEventArgs e) { Nakladka.Log("blad krytyczny: " + e.ExceptionObject); };
            Nakladka.Log("start, rodzic " + parent);
            Application.Run(new Nakladka(parent));
            Nakladka.Log("koniec");
        }
        return 0;
    }
}

/// Ustawienia z indeks.cfg (zapisuje je Indeks - okno ustawien pod zebatka).
sealed class Cfg
{
    public bool Overlay = true, Disks = true, Maximized = true, Enabled = true;
    public int Alpha = 40, DiskAlpha = 15;
    readonly string path = Path.Combine(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "Indeks"), "indeks.cfg");
    DateTime stamp;

    public bool Reload()
    {
        try
        {
            DateTime t = File.GetLastWriteTimeUtc(path);
            if (t == stamp) return false;
            stamp = t;
            foreach (string line in File.ReadAllLines(path, Encoding.UTF8))
            {
                int i = line.IndexOf('=');
                if (i < 0) continue;
                string k = line.Substring(0, i).Trim(), v = line.Substring(i + 1).Trim();
                int n;
                switch (k)
                {
                    case "nakladka": Overlay = v == "1"; break;
                    case "nakladka_dyski": Disks = v == "1"; break;
                    case "nakladka_maks": Maximized = v == "1"; break;
                    case "nakladka_cs": Enabled = v != "0"; break; // tylko jawne 0 wylacza
                    case "nakladka_widocznosc": if (int.TryParse(v, out n)) Alpha = Math.Max(5, Math.Min(100, n)); break;
                    case "dyski_widocznosc": if (int.TryParse(v, out n)) DiskAlpha = Math.Max(5, Math.Min(100, n)); break;
                }
            }
            return true;
        }
        catch { return false; }
    }
}

/// CPU (czasy systemu), GPU (licznik PDH "GPU Engine", silniki 3D - jak Menedzer zadan),
/// RAM i zajetosc dyskow (co 30 s).
sealed class Stats
{
    public double Cpu, Gpu, RamUsed, RamTotal;
    public List<KeyValuePair<string, double>> Disks = new List<KeyValuePair<string, double>>();
    long prevIdle, prevTotal;
    IntPtr query, counter;
    DateTime disksAt = DateTime.MinValue;

    public Stats()
    {
        if (N.PdhOpenQuery(null, IntPtr.Zero, out query) == 0)
        {
            if (N.PdhAddEnglishCounter(query, @"\GPU Engine(*engtype_3D)\Utilization Percentage", IntPtr.Zero, out counter) != 0) counter = IntPtr.Zero;
            N.PdhCollectQueryData(query);
        }
    }

    public void Update()
    {
        long idle, kernel, user;
        if (N.GetSystemTimes(out idle, out kernel, out user))
        {
            long total = kernel + user;
            if (prevTotal > 0 && total > prevTotal)
                Cpu = Math.Max(0, Math.Min(1, 1.0 - (double)(idle - prevIdle) / (total - prevTotal)));
            prevIdle = idle; prevTotal = total;
        }
        var ms = new N.MEMORYSTATUSEX();
        ms.dwLength = (uint)Marshal.SizeOf(typeof(N.MEMORYSTATUSEX));
        if (N.GlobalMemoryStatusEx(ref ms))
        {
            RamTotal = ms.ullTotalPhys;
            RamUsed = ms.ullTotalPhys - ms.ullAvailPhys;
        }
        Gpu = ReadGpu();
        if ((DateTime.Now - disksAt).TotalSeconds > 30)
        {
            disksAt = DateTime.Now;
            var d = new List<KeyValuePair<string, double>>();
            foreach (DriveInfo di in DriveInfo.GetDrives())
            {
                try
                {
                    if (di.DriveType == DriveType.Fixed && di.IsReady && di.TotalSize > 0)
                        d.Add(new KeyValuePair<string, double>(di.Name.TrimEnd('\\'), 1.0 - (double)di.TotalFreeSpace / di.TotalSize));
                }
                catch { }
            }
            Disks = d;
        }
    }

    double ReadGpu()
    {
        if (counter == IntPtr.Zero || N.PdhCollectQueryData(query) != 0) return Gpu;
        uint size = 0, count;
        N.PdhGetFormattedCounterArray(counter, 0x200, ref size, out count, IntPtr.Zero);
        if (size == 0) return 0;
        IntPtr buf = Marshal.AllocHGlobal((int)size);
        try
        {
            if (N.PdhGetFormattedCounterArray(counter, 0x200, ref size, out count, buf) != 0) return Gpu;
            // PDH_FMT_COUNTERVALUE_ITEM_W: wskaznik na nazwe (8) + CStatus (4) + wyrownanie (4) + double (8)
            double sum = 0;
            for (int i = 0; i < count; i++)
                sum += BitConverter.Int64BitsToDouble(Marshal.ReadInt64(buf, i * 24 + 16));
            return Math.Max(0, Math.Min(1, sum / 100.0));
        }
        finally { Marshal.FreeHGlobal(buf); }
    }
}

sealed class Nakladka : Form
{
    readonly Process parent;
    readonly System.Windows.Forms.Timer timer = new System.Windows.Forms.Timer();
    readonly Stats stats = new Stats();
    readonly Cfg cfg = new Cfg();
    readonly N.WinEventProc onForeground;
    DateTime statsAt = DateTime.MinValue;
    string drawn = "";
    bool visible;

    // kolory jak na dolnym pasku Indeksu (CGA)
    static readonly Color LabelColor = Color.FromArgb(0x55, 0xFF, 0xFF), ValueColor = Color.White,
        Green = Color.FromArgb(0x55, 0xFF, 0x55), Yellow = Color.FromArgb(0xFF, 0xFF, 0x55),
        Red = Color.FromArgb(0xFF, 0x55, 0x55), Off = Color.FromArgb(0x3A, 0x3A, 0x3A);

    public Nakladka(int parentPid)
    {
        FormBorderStyle = FormBorderStyle.None;
        ShowInTaskbar = false;
        StartPosition = FormStartPosition.Manual;
        if (parentPid > 0)
        {
            try { parent = Process.GetProcessById(parentPid); }
            catch { Environment.Exit(0); }
        }
        onForeground = delegate { Tick(); LogForeground(); };
        // EVENT_SYSTEM_FOREGROUND: od razu po przelaczeniu aplikacji (Alt+Tab, Start, klik)
        N.SetWinEventHook(3, 3, IntPtr.Zero, onForeground, 0, 0, 0);
        timer.Interval = 500;
        timer.Tick += delegate { Tick(); };
        timer.Start();
    }

    protected override CreateParams CreateParams
    {
        get
        {
            CreateParams cp = base.CreateParams;
            // WS_EX_LAYERED | TRANSPARENT | TOPMOST | TOOLWINDOW | NOACTIVATE
            cp.ExStyle |= 0x80000 | 0x20 | 0x8 | 0x80 | 0x08000000;
            return cp;
        }
    }

    protected override bool ShowWithoutActivation { get { return true; } }

    protected override void SetVisibleCore(bool value)
    {
        // start ukryty - pokazujemy sami tylko nad pelnym ekranem
        if (!IsHandleCreated) { CreateHandle(); value = false; }
        base.SetVisibleCore(value);
    }

    protected override void WndProc(ref Message m)
    {
        if (m.Msg == 0x84) { m.Result = new IntPtr(-1); return; } // WM_NCHITTEST -> HTTRANSPARENT
        base.WndProc(ref m);
    }

    void Tick()
    {
        if (parent != null && parent.HasExited) { Log("Indeks zamkniety - koniec"); Application.Exit(); return; }
        bool cfgChanged = cfg.Reload();
        // w ustawieniach wybrano nakladke w Rust - konczymy (Indeks rysuje sam)
        if (!cfg.Enabled) { Log("nakladka_cs=0 - nakladke rysuje Rust, koniec"); Application.Exit(); return; }
        N.RECT area;
        IntPtr monitor;
        if (!cfg.Overlay || !Detect(out area, out monitor)) { HideOverlay(); return; }
        if ((DateTime.Now - statsAt).TotalMilliseconds >= 1000)
        {
            stats.Update();
            statsAt = DateTime.Now;
        }
        uint dx, dy;
        float s = N.GetDpiForMonitor(monitor, 0, out dx, out dy) == 0 ? dx / 96f : 1f;
        string key = string.Format("{0:F2}|{1:F2}|{2:F2}|{3}|{4}|{5}|{6}|{7}|{8}|{9}", stats.Cpu, stats.Gpu, stats.RamUsed, area.L, area.B, s,
            cfg.Alpha, cfg.DiskAlpha, cfg.Disks, stats.Disks.Count);
        if (key != drawn || cfgChanged || !visible) { Draw(area, s); drawn = key; }
        // gry bez ramki tez bywaja TOPMOST - wracamy na wierzch co tick, bez aktywacji
        N.SetWindowPos(Handle, new IntPtr(-1), 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010 | 0x0040);
        visible = true;
    }

    static readonly string LogPath = Path.Combine(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Indeks"), "nakladka.log");

    /// Dziennik zmian okna na wierzchu: co silnik widzi i czy pokazal nakladke.
    /// stan = SHQueryUserNotificationState: 2 aplikacja na pelnym ekranie, 3 wylaczny pelny
    /// ekran Direct3D (system nie pokaze nad nim zadnego okna), 5 zwykly pulpit.
    void LogForeground()
    {
        try
        {
            IntPtr fg = N.GetForegroundWindow();
            N.RECT r;
            N.GetWindowRect(fg, out r);
            var mi = new N.MONITORINFO();
            mi.cbSize = Marshal.SizeOf(typeof(N.MONITORINFO));
            N.GetMonitorInfo(N.MonitorFromWindow(fg, 2), ref mi);
            int quns;
            N.SHQueryUserNotificationState(out quns);
            uint pid;
            N.GetWindowThreadProcessId(fg, out pid);
            string proc = "?";
            try { proc = Process.GetProcessById((int)pid).ProcessName; } catch { }
            string line = string.Format("{0:yyyy-MM-dd HH:mm:ss} {1} [{2}] okno={3},{4},{5},{6} monitor={7},{8},{9},{10} maks={11} stan={12} nakladka={13}",
                DateTime.Now, proc, ClassOf(fg), r.L, r.T, r.R, r.B, mi.rcMonitor.L, mi.rcMonitor.T, mi.rcMonitor.R, mi.rcMonitor.B,
                N.IsZoomed(fg) ? 1 : 0, quns, visible ? "widoczna" : "ukryta");
            if (quns == 3) line += "  <- wylaczny pelny ekran Direct3D: ustaw w grze tryb 'okno bez ramki' / 'pelny ekran w oknie'";
            Log(line);
        }
        catch { }
    }

    public static void Log(string line)
    {
        try
        {
            var fi = new FileInfo(LogPath);
            if (fi.Exists && fi.Length > 256 * 1024) fi.Delete();
            if (!line.StartsWith("20")) line = DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss ") + line;
            File.AppendAllText(LogPath, line + Environment.NewLine, Encoding.UTF8);
        }
        catch { }
    }

    void HideOverlay()
    {
        if (!visible) return;
        N.ShowWindow(Handle, 0);
        visible = false;
    }

    static string ClassOf(IntPtr h)
    {
        var sb = new StringBuilder(128);
        N.GetClassName(h, sb, sb.Capacity);
        return sb.ToString();
    }

    /// Okno na wierzchu zajmuje caly monitor (pelny ekran) albo jest zmaksymalizowane.
    bool Detect(out N.RECT area, out IntPtr monitor)
    {
        area = new N.RECT();
        monitor = IntPtr.Zero;
        IntPtr fg = N.GetForegroundWindow();
        if (fg == IntPtr.Zero || fg == Handle || !N.IsWindowVisible(fg) || N.IsIconic(fg)) return false;
        switch (ClassOf(fg))
        {
            // pulpit (tam jest pasek Indeksu), pasek zadan, Start, okna Indeksu
            case "Progman": case "WorkerW": case "Shell_TrayWnd": case "Shell_SecondaryTrayWnd":
            case "Windows.UI.Core.CoreWindow": case "NotifyIconOverflowWindow": case "XamlExplorerHostIslandWindow":
            case "IndeksWidok": case "IndeksKontroler": case "IndeksNakladka":
                return false;
        }
        N.RECT r;
        if (!N.GetWindowRect(fg, out r)) return false;
        monitor = N.MonitorFromWindow(fg, 2);
        var mi = new N.MONITORINFO();
        mi.cbSize = Marshal.SizeOf(typeof(N.MONITORINFO));
        if (!N.GetMonitorInfo(monitor, ref mi)) return false;
        N.RECT m = mi.rcMonitor, w = mi.rcWork;
        if (r.L <= m.L && r.T <= m.T && r.R >= m.R && r.B >= m.B) { area = m; return true; }
        if (cfg.Maximized && (N.IsZoomed(fg) || (r.L <= w.L && r.T <= w.T && r.R >= w.R && r.B >= w.B))) { area = w; return true; }
        return false;
    }

    static Color Level(double v) { return v < 0.6 ? Green : v < 0.85 ? Yellow : Red; }

    static string Gb(double b) { return (b / 1073741824.0).ToString("0.0").Replace('.', ','); }

    /// Rysuje jeden rzad: [CPU ▮▮▮ 23%] [GPU ▮▮ 12%] [RAM ▮▮▮▮ 83% 26,5/31,9 GB] [C: ▮▮ 98%]...
    void Draw(N.RECT area, float s)
    {
        using (Bitmap bmp = Render(stats, cfg, s))
            SetBitmap(bmp, area.L + (int)(12 * s), area.B - bmp.Height - (int)(12 * s));
    }

    public static Bitmap Render(Stats stats, Cfg cfg, float s)
    {
        var items = new List<object[]>(); // etykieta, wartosc 0..1, tekst, alfa (0..255)
        int a = cfg.Alpha * 255 / 100, ad = cfg.DiskAlpha * 255 / 100;
        items.Add(new object[] { "CPU", stats.Cpu, Pct(stats.Cpu), a });
        items.Add(new object[] { "GPU", stats.Gpu, Pct(stats.Gpu), a });
        double ram = stats.RamTotal > 0 ? stats.RamUsed / stats.RamTotal : 0;
        items.Add(new object[] { "RAM", ram, Pct(ram) + "  " + Gb(stats.RamUsed) + "/" + Gb(stats.RamTotal) + " GB", a });
        if (cfg.Disks)
            foreach (var d in stats.Disks) items.Add(new object[] { d.Key, d.Value, Pct(d.Value), ad });

        using (var font = new Font("Courier New", 14f * s, FontStyle.Bold, GraphicsUnit.Pixel))
        {
            float pad = 8 * s, seg = 7 * s, segGap = 2 * s, gap = 10 * s, h = (float)Math.Ceiling(28 * s);
            var widths = new List<float>();
            float total = 0;
            using (var tmp = new Bitmap(1, 1))
            using (var g0 = Graphics.FromImage(tmp))
            {
                foreach (object[] it in items)
                {
                    float w = pad + g0.MeasureString((string)it[0], font, PointF.Empty, StringFormat.GenericTypographic).Width + 8 * s
                        + 10 * (seg + segGap) + 8 * s + g0.MeasureString((string)it[2], font, PointF.Empty, StringFormat.GenericTypographic).Width + pad;
                    widths.Add(w);
                    total += w + gap;
                }
            }
            int bw = (int)Math.Ceiling(total), bh = (int)h;
            var bmp = new Bitmap(Math.Max(1, bw), Math.Max(1, bh), System.Drawing.Imaging.PixelFormat.Format32bppArgb);
            {
                using (Graphics g = Graphics.FromImage(bmp))
                {
                    g.Clear(Color.Transparent);
                    g.TextRenderingHint = TextRenderingHint.AntiAliasGridFit;
                    float x = 0;
                    for (int i = 0; i < items.Count; i++)
                    {
                        object[] it = items[i];
                        int al = (int)it[3];
                        double v = (double)it[1];
                        using (var plate = new SolidBrush(Color.FromArgb(al * 85 / 100, 8, 8, 8)))
                            g.FillRectangle(plate, x, 0, widths[i], h);
                        float tx = x + pad, ty = (h - font.Height) / 2f;
                        using (var lb = new SolidBrush(Color.FromArgb(al, LabelColor)))
                            g.DrawString((string)it[0], font, lb, tx, ty, StringFormat.GenericTypographic);
                        tx += g.MeasureString((string)it[0], font, PointF.Empty, StringFormat.GenericTypographic).Width + 8 * s;
                        int on = (int)Math.Round(v * 10);
                        for (int k = 0; k < 10; k++)
                        {
                            using (var sb = new SolidBrush(Color.FromArgb(al, k < on ? Level(v) : Off)))
                                g.FillRectangle(sb, tx, h * 0.25f, seg, h * 0.5f);
                            tx += seg + segGap;
                        }
                        tx += 8 * s;
                        using (var vb = new SolidBrush(Color.FromArgb(al, ValueColor)))
                            g.DrawString((string)it[2], font, vb, tx, ty, StringFormat.GenericTypographic);
                        x += widths[i] + gap;
                    }
                }
                return bmp;
            }
        }
    }

    static string Pct(double v) { return ((int)Math.Round(v * 100)).ToString().PadLeft(3) + "%"; }

    /// Bitmapa z kanalem alfa -> okno warstwowe (UpdateLayeredWindow, alfa na piksel).
    void SetBitmap(Bitmap bmp, int x, int y)
    {
        IntPtr screen = N.GetDC(IntPtr.Zero), mem = N.CreateCompatibleDC(screen);
        IntPtr hbmp = bmp.GetHbitmap(Color.FromArgb(0)), old = N.SelectObject(mem, hbmp);
        try
        {
            var size = new N.SIZE { cx = bmp.Width, cy = bmp.Height };
            var src = new N.POINT();
            var dst = new N.POINT { x = x, y = y };
            var blend = new N.BLENDFUNCTION { BlendOp = 0, SourceConstantAlpha = 255, AlphaFormat = 1 };
            N.UpdateLayeredWindow(Handle, screen, ref dst, ref size, mem, ref src, 0, ref blend, 2);
            N.ShowWindow(Handle, 4); // SW_SHOWNOACTIVATE
        }
        finally
        {
            N.SelectObject(mem, old);
            N.DeleteObject(hbmp);
            N.DeleteDC(mem);
            N.ReleaseDC(IntPtr.Zero, screen);
        }
    }
}

static class N
{
    public delegate void WinEventProc(IntPtr hook, uint ev, IntPtr hwnd, int obj, int child, uint thread, uint time);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int x, y; }
    [StructLayout(LayoutKind.Sequential)] public struct SIZE { public int cx, cy; }
    [StructLayout(LayoutKind.Sequential)] public struct MONITORINFO { public int cbSize; public RECT rcMonitor, rcWork; public uint dwFlags; }
    [StructLayout(LayoutKind.Sequential, Pack = 1)] public struct BLENDFUNCTION { public byte BlendOp, BlendFlags, SourceConstantAlpha, AlphaFormat; }
    [StructLayout(LayoutKind.Sequential)]
    public struct MEMORYSTATUSEX
    {
        public uint dwLength, dwMemoryLoad;
        public ulong ullTotalPhys, ullAvailPhys, ullTotalPageFile, ullAvailPageFile, ullTotalVirtual, ullAvailVirtual, ullAvailExtendedVirtual;
    }

    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] public static extern IntPtr SetWinEventHook(uint min, uint max, IntPtr mod, WinEventProc proc, uint pid, uint tid, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern IntPtr MonitorFromWindow(IntPtr h, uint flags);
    [DllImport("user32.dll")] public static extern bool GetMonitorInfo(IntPtr m, ref MONITORINFO mi);
    [DllImport("shcore.dll")] public static extern int GetDpiForMonitor(IntPtr m, int type, out uint x, out uint y);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool UpdateLayeredWindow(IntPtr h, IntPtr dst, ref POINT pt, ref SIZE sz, IntPtr src, ref POINT spt, int key, ref BLENDFUNCTION blend, int flags);
    [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr dc);
    [DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr dc);
    [DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr dc, IntPtr obj);
    [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr obj);
    [DllImport("shell32.dll")] public static extern int SHQueryUserNotificationState(out int state);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("kernel32.dll")] public static extern bool GetSystemTimes(out long idle, out long kernel, out long user);
    [DllImport("kernel32.dll")] public static extern bool GlobalMemoryStatusEx(ref MEMORYSTATUSEX ms);
    [DllImport("pdh.dll", CharSet = CharSet.Unicode)] public static extern uint PdhOpenQuery(string src, IntPtr user, out IntPtr q);
    [DllImport("pdh.dll", CharSet = CharSet.Unicode)] public static extern uint PdhAddEnglishCounter(IntPtr q, string path, IntPtr user, out IntPtr c);
    [DllImport("pdh.dll")] public static extern uint PdhCollectQueryData(IntPtr q);
    [DllImport("pdh.dll", CharSet = CharSet.Unicode)] public static extern uint PdhGetFormattedCounterArray(IntPtr c, uint fmt, ref uint size, out uint count, IntPtr buf);
}
