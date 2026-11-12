// Indeks - dane panelu: wczytanie katalogu zapisanego przez silnik C++ (cpp/katalog.h).
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Globalization;
using System.IO;
using System.Text;

sealed class Lang
{
    public readonly string Name;
    public readonly Color Color;
    public Lang(string name, int rgb)
    {
        Name = name;
        int r = (rgb >> 16) & 255, g = (rgb >> 8) & 255, b = rgb & 255;
        // zbyt ciemne kolory GitHuba rozjasnione, zeby byly widoczne na czarnym tle
        if (0.299 * r + 0.587 * g + 0.114 * b < 90) { r = (r + 0xB0) / 2; g = (g + 0xB0) / 2; b = (b + 0xB0) / 2; }
        Color = Color.FromArgb(r, g, b);
    }
}

sealed class Category
{
    public readonly string Name, Restore;
    public readonly Color Color;
    public Category(string name, int rgb, string restore)
    {
        Name = name;
        Color = Color.FromArgb((rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
        Restore = restore;
    }
}

static class Tables
{
    // Kolejnosc = identyfikatory z src/profiles.rs (LANGS).
    public static readonly Lang[] Langs = {
        new Lang("JavaScript", 0xF1E05A), new Lang("TypeScript", 0x3178C6), new Lang("Rust", 0xDEA584), new Lang("C++", 0xF34B7D),
        new Lang("C", 0x555555), new Lang("C#", 0x178600), new Lang("Python", 0x3572A5), new Lang("Go", 0x00ADD8),
        new Lang("Java", 0xB07219), new Lang("Kotlin", 0xA97BFF), new Lang("PHP", 0x4F5D95), new Lang("Ruby", 0x701516),
        new Lang("HTML", 0xE34C26), new Lang("CSS", 0x563D7C), new Lang("Shell", 0x89E051), new Lang("Lua", 0x000080),
        new Lang("Dart", 0x00B4AB), new Lang("Swift", 0xF05138), new Lang("MQL", 0x4A90D9), new Lang("Pine Script", 0x2962FF),
        new Lang("Vue", 0x41B883), new Lang("Svelte", 0xFF3E00), new Lang("SQL", 0xE38C00), new Lang("Assembly", 0x6E4C13),
        new Lang("Zig", 0xEC915C), new Lang("R", 0x198CE7), new Lang("Scala", 0xC22D40), new Lang("Haskell", 0x5E5086),
        new Lang("Julia", 0xA270BA), new Lang("WebAssembly", 0x04133B), new Lang("Shadery", 0x5686A5), new Lang("Node.js", 0x339933),
    };

    // Kolejnosc = enum Category z cpp/katalog.h.
    public static readonly Category[] Cats = {
        new Category("Biblioteki Node", 0x66BB6A, "npm install"),
        new Category("Rust target", 0xDEA584, "cargo build"),
        new Category("Debug / Release C++", 0xF34B7D, "przebuduj projekt"),
        new Category(".NET bin / obj", 0x7FBF7F, "dotnet build"),
        new Category("Python", 0x5B9BD5, "pip install"),
        new Category("Wyniki budowania web", 0xF1E05A, "npm run build"),
        new Category("Java / Gradle / PHP", 0xB07219, "przebuduj projekt"),
        new Category("Temp systemu", 0x9E9EFF, "nie trzeba przywracać"),
        new Category("Claude i Codex", 0xFFD84D, "narzędzie utworzy na nowo"),
        new Category("Pamięć podręczna pakietów", 0x55FFFF, "pobierze się przy instalacji"),
        new Category("Logi i zrzuty", 0xFF8A8A, "nie trzeba przywracać"),
    };
}

sealed class FileEntry
{
    public string Rel;
    public long Bytes;
    public uint Time;
}

sealed class TrashEntry
{
    public string Path, Project;
    public int Cat, Safety, Files;
    public long Bytes;
    public uint Newest;
    public List<FileEntry> Top = new List<FileEntry>();
    public bool Selected, Open;

    public string Name
    {
        get
        {
            string p = Path.EndsWith("\\*") ? Path.Substring(0, Path.Length - 2) : Path;
            int i = p.LastIndexOf('\\');
            string n = i >= 0 ? p.Substring(i + 1) : p;
            return Path.EndsWith("\\*") ? n + " (pliki luzem)" : n;
        }
    }
}

sealed class Project
{
    public string Path;
    public int Kind, Files;
    public long Bytes, Trash;
    public uint Newest;
    public bool Git, Open;
    public List<KeyValuePair<int, long>> Langs = new List<KeyValuePair<int, long>>();
    public List<TrashEntry> Items = new List<TrashEntry>();

    public long Total { get { return Bytes + Trash; } }
    public string Name { get { int i = Path.LastIndexOf('\\'); return i >= 0 ? Path.Substring(i + 1) : Path; } }
}

sealed class Catalog
{
    public uint ScanMin, Changed;
    public long Ms;
    public int Dirs;
    public List<Project> Projects = new List<Project>();
    public List<TrashEntry> Trash = new List<TrashEntry>();

    static long L(string s) { long v; long.TryParse(s, NumberStyles.None, CultureInfo.InvariantCulture, out v); return v; }
    static int I(string s) { return (int)L(s); }
    static uint U(string s) { return (uint)L(s); }

    public static Catalog Load(string file)
    {
        var c = new Catalog();
        if (!File.Exists(file)) return null;
        string[] lines;
        try { lines = File.ReadAllLines(file, new UTF8Encoding(false)); }
        catch (IOException) { return null; }
        TrashEntry last = null;
        var byPath = new Dictionary<string, Project>(StringComparer.OrdinalIgnoreCase);
        // zrzuty do README: bez nazw, ktore na publicznym zdjeciu wygladalyby zle
        bool demo = Environment.GetEnvironmentVariable("INDEKS_DEMO") != null;
        string[] demoWords = { "steal", "cheat", "token", "grab", "virus", "wirus", "inject", "keydrop", "loader", "hack", "crack", "keylog", "spoof", "bypass", "exploit", "malware", "nuker", "miner", "rat_", "obf", "_load" };
        foreach (string line in lines)
        {
            if (line.Length < 2) continue;
            if (demo && line[0] != 'V')
            {
                string low = line.ToLowerInvariant();
                bool bad = false;
                foreach (string w in demoWords) if (low.Contains(w)) { bad = true; break; }
                if (bad) { if (line[0] == 'T') last = null; continue; }
            }
            string[] f = line.Split('\t');
            switch (line[0])
            {
                case 'V':
                    if (f.Length >= 6) { c.ScanMin = U(f[2]); c.Ms = L(f[3]); c.Dirs = I(f[4]); c.Changed = U(f[5]); }
                    break;
                case 'P':
                    if (f.Length < 8) break;
                    var p = new Project { Path = f[1], Kind = I(f[2]), Bytes = L(f[3]), Files = I(f[4]), Newest = U(f[5]), Git = f[6] == "1" };
                    foreach (string part in f[7].Split(','))
                    {
                        int k = part.IndexOf(':');
                        if (k > 0) p.Langs.Add(new KeyValuePair<int, long>(I(part.Substring(0, k)), L(part.Substring(k + 1))));
                    }
                    p.Langs.Sort((a, b) => b.Value.CompareTo(a.Value));
                    c.Projects.Add(p);
                    byPath[p.Path] = p;
                    last = null;
                    break;
                case 'T':
                    if (f.Length < 8) break;
                    last = new TrashEntry { Path = f[1], Cat = I(f[2]), Safety = I(f[3]), Bytes = L(f[4]), Files = I(f[5]), Newest = U(f[6]), Project = f[7] };
                    c.Trash.Add(last);
                    break;
                case 'F':
                    if (last != null && f.Length >= 4) last.Top.Add(new FileEntry { Rel = f[1], Bytes = L(f[2]), Time = U(f[3]) });
                    break;
            }
        }
        foreach (TrashEntry t in c.Trash)
        {
            Project p;
            if (t.Project.Length > 0 && byPath.TryGetValue(t.Project, out p)) { p.Trash += t.Bytes; p.Items.Add(t); }
            // od najstarszych; w tym samym dniu najciezsze pierwsze
            t.Top.Sort((a, b) => { int d = (a.Time / 1440).CompareTo(b.Time / 1440); return d != 0 ? d : b.Bytes.CompareTo(a.Bytes); });
        }
        return c;
    }
}

static class Fmt
{
    public static string Size(long b)
    {
        var pl = CultureInfo.GetCultureInfo("pl-PL");
        if (b >= 1L << 40) return (b / (double)(1L << 40)).ToString("0.00", pl) + " TB";
        if (b >= 1L << 30) return (b / (double)(1L << 30)).ToString("0.0", pl) + " GB";
        if (b >= 1L << 20) return (b / (double)(1L << 20)).ToString("0", pl) + " MB";
        return Math.Max(1, b / 1024).ToString(pl) + " KB";
    }

    public static string Count(long n)
    {
        return n.ToString("#,0", CultureInfo.GetCultureInfo("pl-PL"));
    }

    public static uint Now()
    {
        return (uint)(DateTime.UtcNow.ToFileTimeUtc() / 600000000L);
    }

    public static string Ago(uint min)
    {
        if (min == 0) return "";
        uint d = Now() > min ? Now() - min : 0, days = d / 1440;
        if (d < 2) return "przed chwilą";
        if (d < 60) return d + " min temu";
        if (d < 1440) return d / 60 + " godz. temu";
        if (days == 1) return "wczoraj";
        if (days < 30) return days + " dni temu";
        if (days < 365) return days / 30 + " mies. temu";
        uint y = days / 365;
        return y + (y == 1 ? " rok" : (y % 10 >= 2 && y % 10 <= 4 && (y % 100 < 12 || y % 100 > 14)) ? " lata" : " lat") + " temu";
    }

    public static string Plural(long n, string one, string few, string many)
    {
        string w = n == 1 ? one : (n % 10 >= 2 && n % 10 <= 4 && (n % 100 < 12 || n % 100 > 14)) ? few : many;
        return Count(n) + " " + w;
    }
}
