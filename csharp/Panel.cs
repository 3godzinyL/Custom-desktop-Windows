// Indeks - panel "Projekty na dysku" (C#): wyswietla gotowy katalog z silnika C++.
// Proces startuje w tle z ukrytym oknem i wczytanym katalogiem, wiec klikniecie tylko je pokazuje.
// Rust wstawia okno jako dziecko widoku (SetParent) i ustawia jego polozenie.
//   indeks-panel.exe --rodzic PID --sterownik HWND
//   indeks-panel.exe --zrzut plik.png [--szer W] [--wys H] [--dpi D] [--widok 0|1]
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Forms;

static class Native
{
    [StructLayout(LayoutKind.Sequential)]
    public struct CopyData { public IntPtr Data; public int Len; public IntPtr Ptr; }
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr h);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessageTimeoutW(IntPtr h, int msg, IntPtr wp, ref CopyData lp, int flags, int timeout, out IntPtr res);
    [DllImport("kernel32.dll")] public static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll")] public static extern bool SetProcessWorkingSetSize(IntPtr p, IntPtr min, IntPtr max);
    public const int WM_COPYDATA = 0x4A, WM_SHOWWINDOW = 0x18;
}

static class Program
{
    static string Arg(string[] a, string name, string def)
    {
        for (int i = 0; i + 1 < a.Length; i++) if (a[i] == name) return a[i + 1];
        return def;
    }

    [STAThread]
    static int Main(string[] args)
    {
        try { Native.SetProcessDpiAwarenessContext(new IntPtr(-4)); } catch { Native.SetProcessDPIAware(); }
        if (args.Length >= 2 && args[0] == "--zrzut")
        {
            var p = new Panel(IntPtr.Zero, 0);
            int w = int.Parse(Arg(args, "--szer", "1800")), h = int.Parse(Arg(args, "--wys", "1100"));
            p.Scale1 = float.Parse(Arg(args, "--dpi", "144")) / 96f;
            p.View = int.Parse(Arg(args, "--widok", "0"));
            p.ReloadFonts();
            using (var b = new Bitmap(w, h))
            {
                using (var g = Graphics.FromImage(b)) p.Draw(g, w, h);
                b.Save(args[1]);
            }
            return 0;
        }
        int parent = int.Parse(Arg(args, "--rodzic", "0"));
        long ctl = long.Parse(Arg(args, "--sterownik", "0"));
        // jeden panel na proces Indeksu
        bool created;
        var single = new Mutex(true, @"Local\Indeks.Panel." + parent, out created);
        if (!created) return 0;
        GC.KeepAlive(single);
        Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);
        Application.ThreadException += (s, e) => Panel.Log("wyjatek: " + e.Exception);
        AppDomain.CurrentDomain.UnhandledException += (s, e) => Panel.Log("blad krytyczny: " + e.ExceptionObject);
        var panel = new Panel(new IntPtr(ctl), parent);
        panel.CreateHidden();
        if (parent != 0)
        {
            var t = new Thread(() =>
            {
                try { Process.GetProcessById(parent).WaitForExit(); } catch (ArgumentException) { }
                Environment.Exit(0);
            });
            t.IsBackground = true;
            t.Start();
        }
        panel.HandleDestroyed += (s, e) => Application.ExitThread();
        panel.Ready();
        Application.Run();
        return 0;
    }
}

sealed partial class Panel : Form
{
    static readonly Color BG = Rgb(0x0A0A0A), FRAME = Rgb(0x4E4E4E), TEXT = Rgb(0xFFFFFF), LINK = Rgb(0x9E9EFF), DIM = Rgb(0x8C8C8C),
        ERR = Rgb(0xFF8A8A), YELLOW = Rgb(0xFFD84D), STRIPE = Rgb(0x111116), HOVER = Rgb(0x16163A), GREEN = Rgb(0x55FF55), BAR = Rgb(0x232323);
    const int COPY_OPEN = 0x494E4458, COPY_PANEL = 0x494E445A, COPY_TO_PANEL = 0x50414E4C;
    const TextFormatFlags TF = TextFormatFlags.NoPrefix | TextFormatFlags.SingleLine | TextFormatFlags.NoPadding | TextFormatFlags.EndEllipsis | TextFormatFlags.Top;

    static Color Rgb(int v) { return Color.FromArgb((v >> 16) & 255, (v >> 8) & 255, v & 255); }

    static readonly string Dir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Indeks");
    static readonly string CatalogFile = Path.Combine(Dir, "katalog.tsv");

    public static void Log(string s)
    {
        try { File.AppendAllText(Path.Combine(Dir, "panel.log"), DateTime.Now.ToString("HH:mm:ss ") + s + "\r\n"); } catch (IOException) { }
    }

    IntPtr ctl;
    readonly int parentPid;
    Catalog cat;
    DateTime catStamp;
    public float Scale1 = 1f;
    public int View;
    Font fBody, fBold, fMono, fMonoBold, fTitle;
    int rowH;

    // stan listy
    float scroll;
    int drawW, contentH, viewTop, viewH;
    bool dragThumb;
    int dragY;
    float dragScroll;
    string projectSort = "size";
    bool projectAsc;
    string cleanSort = "age";
    bool cleanAsc = true;
    string cleanPreset = "all";
    int langFilter = -1;
    string projectFilter;
    readonly HashSet<int> hiddenCats = new HashSet<int>();

    // usuwanie / skan
    bool confirm;
    Process remover;
    string status = "";
    string progressText = "";
    double progressFrac = -1;
    readonly System.Windows.Forms.Timer timer = new System.Windows.Forms.Timer();

    struct HitBox { public Rectangle R; public Action Do; }
    readonly List<HitBox> hits = new List<HitBox>();
    Point mouse = new Point(-1, -1);
    int hoverIndex = -1;

    public Panel(IntPtr ctl, int parentPid)
    {
        this.ctl = ctl;
        this.parentPid = parentPid;
        FormBorderStyle = FormBorderStyle.None;
        ShowInTaskbar = false;
        StartPosition = FormStartPosition.Manual;
        BackColor = BG;
        Text = "Indeks.Panel." + parentPid;
        SetStyle(ControlStyles.OptimizedDoubleBuffer | ControlStyles.AllPaintingInWmPaint | ControlStyles.UserPaint | ControlStyles.Opaque, true);
        ReloadFonts();
        LoadCatalog();
        timer.Interval = 700;
        timer.Tick += (s, e) => Tick();
    }

    public void CreateHidden()
    {
        CreateHandle();
        Scale1 = Math.Max(1f, Native.GetDpiForWindow(Handle) / 96f);
        ReloadFonts();
    }

    public void Ready()
    {
        Send(COPY_PANEL, "gotowy");
        // do pierwszego otwarcia proces czeka w tle prawie bez pamieci roboczej
        GC.Collect();
        Native.SetProcessWorkingSetSize(Native.GetCurrentProcess(), new IntPtr(-1), new IntPtr(-1));
    }

    public void ReloadFonts()
    {
        fBody = new Font("Times New Roman", 16f * Scale1, GraphicsUnit.Pixel);
        fBold = new Font("Times New Roman", 16f * Scale1, FontStyle.Bold, GraphicsUnit.Pixel);
        fTitle = new Font("Times New Roman", 22f * Scale1, FontStyle.Bold, GraphicsUnit.Pixel);
        fMono = new Font("Courier New", 13f * Scale1, GraphicsUnit.Pixel);
        fMonoBold = new Font("Courier New", 13f * Scale1, FontStyle.Bold, GraphicsUnit.Pixel);
        rowH = S(24);
    }

    int S(float v) { return (int)Math.Round(v * Scale1); }

    void LoadCatalog()
    {
        var selected = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        if (cat != null) foreach (var t in cat.Trash) if (t.Selected) selected.Add(t.Path);
        Catalog c = Catalog.Load(CatalogFile);
        if (c == null) return;
        foreach (var t in c.Trash) t.Selected = selected.Contains(t.Path);
        cat = c;
        catStamp = File.GetLastWriteTimeUtc(CatalogFile);
    }

    // ---------- komunikaty ----------

    protected override void WndProc(ref Message m)
    {
        if (m.Msg == Native.WM_COPYDATA)
        {
            var cd = (Native.CopyData)Marshal.PtrToStructure(m.LParam, typeof(Native.CopyData));
            if (cd.Data.ToInt64() == COPY_TO_PANEL && cd.Ptr != IntPtr.Zero) Command(Marshal.PtrToStringUni(cd.Ptr, cd.Len / 2).TrimEnd('\0'));
            m.Result = new IntPtr(1);
            return;
        }
        if (m.Msg == Native.WM_SHOWWINDOW)
        {
            if (m.WParam == IntPtr.Zero)
            {
                timer.Stop();
                // ukryty panel oddaje pamiec robocza; katalog zostaje gotowy
                GC.Collect();
                Native.SetProcessWorkingSetSize(Native.GetCurrentProcess(), new IntPtr(-1), new IntPtr(-1));
            }
            else
            {
                float s = Math.Max(1f, Native.GetDpiForWindow(Handle) / 96f);
                if (Math.Abs(s - Scale1) > 0.01f) { Scale1 = s; ReloadFonts(); }
                timer.Start();
                Tick();
            }
        }
        base.WndProc(ref m);
    }

    void Command(string line)
    {
        string[] f = line.Split('\t');
        switch (f[0])
        {
            case "wczytaj": LoadCatalog(); break;
            case "wlasciciel": if (f.Length > 1) ctl = new IntPtr(long.Parse(f[1])); break;
            case "jezyk": if (f.Length > 1) { langFilter = int.Parse(f[1]); scroll = 0; } break;
            case "projekt": if (f.Length > 1) ShowProject(f[1]); break;
        }
        Invalidate();
    }

    void Send(int kind, string text)
    {
        if (ctl == IntPtr.Zero) return;
        byte[] data = Encoding.Unicode.GetBytes(text + "\0");
        IntPtr mem = Marshal.AllocHGlobal(data.Length);
        try
        {
            Marshal.Copy(data, 0, mem, data.Length);
            var cd = new Native.CopyData { Data = new IntPtr(kind), Len = data.Length, Ptr = mem };
            IntPtr res;
            Native.SendMessageTimeoutW(ctl, Native.WM_COPYDATA, Handle, ref cd, 2, 3000, out res);
        }
        finally { Marshal.FreeHGlobal(mem); }
    }

    void ShowProject(string path)
    {
        if (cat == null) return;
        if (View == 1) { projectFilter = path; scroll = 0; return; }
        foreach (var p in cat.Projects) p.Open = string.Equals(p.Path, path, StringComparison.OrdinalIgnoreCase);
        var rows = SortedProjects();
        int y = 0;
        foreach (var p in rows)
        {
            if (p.Open) { scroll = y; break; }
            y += rowH;
        }
    }

    void Tick()
    {
        string file = CatalogFile + ".postep";
        string old = progressText;
        double oldFrac = progressFrac;
        progressText = "";
        progressFrac = -1;
        try
        {
            if (File.Exists(file))
            {
                string[] f = File.ReadAllText(file, Encoding.UTF8).Split('\t');
                if (f[0] == "skan" && f.Length >= 4)
                    progressText = "Silnik C++ przeszukuje dysk: " + Fmt.Plural(long.Parse(f[1]), "folder", "foldery", "folderów") + ", " + Fmt.Plural(long.Parse(f[2]), "plik", "pliki", "plików") + ", " + Fmt.Size(long.Parse(f[3]));
                else if (f[0] == "usun" && f.Length >= 4)
                {
                    long done = long.Parse(f[1]), total = Math.Max(1, long.Parse(f[2]));
                    progressFrac = Math.Min(1.0, done / (double)total);
                    progressText = "Usuwanie: " + Fmt.Size(done) + " z " + Fmt.Size(total) + "  ·  " + f[3];
                }
            }
        }
        catch (IOException) { }
        catch (FormatException) { }
        if (remover != null && remover.HasExited) FinishRemoval();
        DateTime stamp = File.Exists(CatalogFile) ? File.GetLastWriteTimeUtc(CatalogFile) : catStamp;
        bool changed = stamp != catStamp;
        if (changed) LoadCatalog();
        if (changed || old != progressText || oldFrac != progressFrac) Invalidate();
    }

    // ---------- usuwanie ----------

    void StartRemoval()
    {
        confirm = false;
        var list = cat.Trash.Where(t => t.Selected).Select(t => t.Path).ToList();
        if (list.Count == 0) return;
        string listFile = Path.Combine(Dir, "katalog.usun");
        File.WriteAllLines(listFile, list, new UTF8Encoding(false));
        string exe = Path.Combine(Path.GetDirectoryName(Application.ExecutablePath), "indeks-katalog.exe");
        try
        {
            remover = Process.Start(new ProcessStartInfo(exe, "--usun \"" + listFile + "\"") { UseShellExecute = false, CreateNoWindow = true });
            status = "";
        }
        catch (System.ComponentModel.Win32Exception e) { status = "Nie można uruchomić silnika: " + e.Message; }
    }

    void FinishRemoval()
    {
        remover = null;
        try
        {
            string[] f = File.ReadAllText(CatalogFile + ".wynik").Split('\t');
            long freed = long.Parse(f[0]), failed = long.Parse(f[1]);
            status = "Zwolniono " + Fmt.Size(freed) + (failed > 0 ? "  ·  " + Fmt.Plural(failed, "element był zablokowany", "elementy były zablokowane", "elementów było zablokowanych") + " (w użyciu) i został" : "");
        }
        catch (IOException) { status = "Usuwanie zakończone."; }
        LoadCatalog();
        Send(COPY_PANEL, "wczytaj");
    }

    void SelectSafeOld()
    {
        uint limit = Fmt.Now() - 30u * 1440u;
        foreach (var t in VisibleTrash()) t.Selected = t.Safety == 1 && t.Newest != 0 && t.Newest < limit;
    }

    // ---------- listy ----------

    List<Project> SortedProjects()
    {
        if (cat == null) return new List<Project>();
        IEnumerable<Project> q = cat.Projects;
        if (langFilter >= 0) q = q.Where(p => p.Kind == langFilter);
        Func<Project, IComparable> key;
        switch (projectSort)
        {
            case "name": key = p => p.Name.ToLowerInvariant(); break;
            case "trash": key = p => p.Trash; break;
            case "date": key = p => p.Newest; break;
            default: key = p => p.Total; break;
        }
        return (projectAsc ? q.OrderBy(key) : q.OrderByDescending(key)).ToList();
    }

    IEnumerable<TrashEntry> VisibleTrash()
    {
        if (cat == null) return Enumerable.Empty<TrashEntry>();
        IEnumerable<TrashEntry> q = cat.Trash.Where(t => !hiddenCats.Contains(t.Cat));
        if (projectFilter != null) q = q.Where(t => string.Equals(t.Project, projectFilter, StringComparison.OrdinalIgnoreCase));
        return q.Where(MatchesPreset);
    }

    // Presety: szybki wybór rodzaju śmieci (nie każdy da się opisać idealnie automatycznie).
    bool MatchesPreset(TrashEntry t)
    {
        switch (cleanPreset)
        {
            case "build": return t.Cat <= 6 || t.Cat == 9;              // biblioteki + wyniki budowania + pakiety
            case "ai": return t.Cat == 7 || t.Cat == 8;                 // Temp systemu + Claude/Codex
            case "logs": return t.Cat == 10;
            case "files": return t.Path.EndsWith("\\*");                // pliki luzem (Temp)
            case "old": return t.Newest != 0 && Fmt.Now() - t.Newest > 90u * 1440u;  // dawno niemodyfikowane
            default: return true;
        }
    }

    List<TrashEntry> SortedTrash()
    {
        // domyslnie od najstarszych: to one sa najpewniejsze do usuniecia
        IEnumerable<TrashEntry> q = VisibleTrash();
        if (cleanSort == "size") q = cleanAsc ? q.OrderBy(t => t.Bytes) : q.OrderByDescending(t => t.Bytes);
        else q = cleanAsc ? q.OrderBy(t => t.Newest / 1440).ThenByDescending(t => t.Bytes) : q.OrderByDescending(t => t.Newest / 1440).ThenByDescending(t => t.Bytes);
        return q.ToList();
    }

    protected override void OnPaint(PaintEventArgs e) { Draw(e.Graphics, ClientSize.Width, ClientSize.Height); }
    protected override void OnPaintBackground(PaintEventArgs e) { }

    // ---------- mysz ----------

    int HitAt(Point p)
    {
        for (int i = hits.Count - 1; i >= 0; i--) if (hits[i].R.Contains(p)) return i;
        return -1;
    }

    Rectangle ThumbRect()
    {
        int x = drawW - S(18), track = viewH;
        if (contentH <= viewH || track <= 0) return Rectangle.Empty;
        int th = Math.Max(S(24), track * viewH / contentH);
        int y = viewTop + (int)((track - th) * (scroll / Math.Max(1f, contentH - viewH)));
        return new Rectangle(x, y, S(10), th);
    }

    void ClampScroll() { scroll = Math.Max(0, Math.Min(scroll, Math.Max(0, contentH - viewH))); }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        mouse = e.Location;
        if (dragThumb)
        {
            int th = ThumbRect().Height, range = Math.Max(1, viewH - th);
            scroll = dragScroll + (e.Y - dragY) * (float)(contentH - viewH) / range;
            ClampScroll();
            Invalidate();
            return;
        }
        int h = HitAt(e.Location);
        Cursor = h >= 0 ? Cursors.Hand : Cursors.Default;
        if (h != hoverIndex) { hoverIndex = h; Invalidate(); }
    }

    protected override void OnMouseLeave(EventArgs e) { mouse = new Point(-1, -1); hoverIndex = -1; Invalidate(); }

    protected override void OnMouseDown(MouseEventArgs e)
    {
        Native.SetFocus(Handle);
        Rectangle t = ThumbRect();
        if (!confirm && e.Button == MouseButtons.Left && t.Width > 0 && e.X >= t.X)
        {
            if (t.Contains(e.Location)) { dragThumb = true; dragY = e.Y; dragScroll = scroll; }
            else { scroll += (e.Y < t.Y ? -1 : 1) * viewH * 0.9f; ClampScroll(); Invalidate(); }
            return;
        }
        if (e.Button != MouseButtons.Left) return;
        int h = HitAt(e.Location);
        if (h >= 0) { hits[h].Do(); hoverIndex = -1; Invalidate(); }
    }

    protected override void OnMouseUp(MouseEventArgs e) { dragThumb = false; }

    protected override void OnMouseWheel(MouseEventArgs e)
    {
        if (confirm) return;
        scroll -= e.Delta / 120f * S(100);
        ClampScroll();
        hoverIndex = -1;
        Invalidate();
    }

    protected override bool ProcessDialogKey(Keys k)
    {
        if (k == Keys.Escape && confirm) { confirm = false; Invalidate(); return true; }
        return base.ProcessDialogKey(k);
    }
}
