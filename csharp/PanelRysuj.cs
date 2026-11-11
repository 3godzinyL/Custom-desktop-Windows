// Indeks - rysowanie panelu: widok wszystkich projektow i widok czyszczenia dysku.
using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Windows.Forms;

sealed partial class Panel
{
    void Txt(Graphics g, string s, int x, int y, int w, Font f, Color c)
    {
        if (w > 0) TextRenderer.DrawText(g, s, f, new Rectangle(x, y, w, f.Height + S(2)), c, TF);
    }

    void TxtRight(Graphics g, string s, int right, int y, int w, Font f, Color c)
    {
        TextRenderer.DrawText(g, s, f, new Rectangle(right - w, y, w, f.Height + S(2)), c, TF | TextFormatFlags.Right);
    }

    // bez EndEllipsis: przy pustym rozmiarze skracalby tekst do "W..."
    int Measure(Graphics g, string s, Font f) { return TextRenderer.MeasureText(g, s, f, new Size(int.MaxValue, int.MaxValue), TextFormatFlags.NoPrefix | TextFormatFlags.SingleLine | TextFormatFlags.NoPadding).Width; }

    void Fill(Graphics g, Color c, int x, int y, int w, int h)
    {
        using (var b = new SolidBrush(c)) g.FillRectangle(b, x, y, w, h);
    }

    void Hit(Rectangle r, Action a) { hits.Add(new HitBox { R = r, Do = a }); }
    bool Hot(Rectangle r) { return !confirm && r.Contains(mouse); }

    // Link jak w reszcie Indeksu: kolor linku, podkreslenie po najechaniu.
    int Link(Graphics g, string s, int x, int y, Font f, Color c, Action a)
    {
        int w = Measure(g, s, f);
        var r = new Rectangle(x, y, w, f.Height);
        bool hot = Hot(r);
        Txt(g, s, x, y, w + S(2), f, c);
        if (hot) Fill(g, c, x, y + f.Height - S(2), w, Math.Max(1, S(1)));
        Hit(r, a);
        return w;
    }

    void Frame(Graphics g, int W, int H, string title)
    {
        int p = Math.Max(1, S(1)), k = 3 * p;
        foreach (int o in new[] { 0, k })
        {
            Fill(g, FRAME, o, o, W - 2 * o, p);
            Fill(g, FRAME, o, H - p - o, W - 2 * o, p);
            Fill(g, FRAME, o, o, p, H - 2 * o);
            Fill(g, FRAME, W - p - o, o, p, H - 2 * o);
        }
    }

    void Dot(Graphics g, Color c, int cx, int cy)
    {
        int r = S(5);
        var old = g.SmoothingMode;
        g.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
        using (var b = new SolidBrush(c)) g.FillEllipse(b, cx - r, cy - r, 2 * r, 2 * r);
        g.SmoothingMode = old;
    }

    void LangBar(Graphics g, int x, int y, int w, int h, IList<KeyValuePair<int, long>> langs)
    {
        long total = langs.Sum(l => l.Value);
        if (total <= 0) { Fill(g, BAR, x, y, w, h); return; }
        int sx = x;
        for (int i = 0; i < langs.Count; i++)
        {
            int pw = i + 1 == langs.Count ? x + w - sx : (int)(w * (double)langs[i].Value / total);
            if (pw > 0) Fill(g, Tables.Langs[langs[i].Key].Color, sx, y, pw, h);
            sx += pw;
        }
    }

    public void Draw(Graphics g, int W, int H)
    {
        hits.Clear();
        drawW = W;
        g.Clear(BG);
        Frame(g, W, H, "");
        int pad = S(14), y = S(10);
        // przelacznik widokow
        int x = pad;
        x += Switch(g, "Wszystkie projekty", x, y, 0) + S(26);
        x += Switch(g, "Czyszczenie dysku", x, y, 1) + S(26);
        if (cat != null)
        {
            int right = W - pad;
            string rescan = "[Skanuj ponownie]";
            int rw = Measure(g, rescan, fBody);
            Link(g, rescan, right - rw, y, fBody, LINK, () => Send(COPY_PANEL, "skanuj"));
            string info = "katalog: " + Fmt.Ago(cat.ScanMin) + "  ·  " + Fmt.Plural(cat.Dirs, "folder", "foldery", "folderów");
            int iw = Measure(g, info, fMono);
            Txt(g, info, right - rw - S(16) - iw, y + S(3), iw + S(2), fMono, DIM);
        }
        y += S(32);
        Fill(g, FRAME, pad, y - S(4), W - 2 * pad, Math.Max(1, S(1)));
        if (progressText.Length > 0)
        {
            Txt(g, progressText, pad, y + S(2), W - 2 * pad, fBody, YELLOW);
            y += S(26);
            if (progressFrac >= 0)
            {
                Fill(g, BAR, pad, y, W - 2 * pad, S(8));
                Fill(g, GREEN, pad, y, (int)((W - 2 * pad) * progressFrac), S(8));
                y += S(14);
            }
        }
        if (cat == null)
        {
            Txt(g, progressText.Length > 0 ? "Pierwsze przeszukanie trwa kilka minut, potem panel otwiera się od razu." : "Brak katalogu. Silnik C++ uruchomi się za chwilę.", pad, y + S(10), W - 2 * pad, fBody, DIM);
            return;
        }
        if (View == 0) DrawProjects(g, W, H, y); else DrawClean(g, W, H, y);
        DrawScroll(g);
        if (confirm) DrawConfirm(g, W, H);
    }

    int Switch(Graphics g, string s, int x, int y, int view)
    {
        if (View == view)
        {
            int w = Measure(g, s, fBold);
            Txt(g, s, x, y, w + S(2), fBold, YELLOW);
            Fill(g, YELLOW, x, y + fBold.Height + S(1), w, Math.Max(1, S(2)));
            return w;
        }
        return Link(g, s, x, y, fBody, LINK, () => { View = view; scroll = 0; });
    }

    void DrawScroll(Graphics g)
    {
        Rectangle t = ThumbRect();
        if (t.Width == 0) return;
        Fill(g, BAR, t.X, viewTop, t.Width, viewH);
        Fill(g, dragThumb || t.Contains(mouse) ? DIM : FRAME, t.X, t.Y, t.Width, t.Height);
    }

    void Header(Graphics g, string s, int x, int y, int w, bool right, bool active, bool asc, Action a)
    {
        string label = active ? s + (asc ? " ↑" : " ↓") : s;
        int tw = Measure(g, label, fMonoBold);
        int tx = right ? x + w - tw : x;
        var r = new Rectangle(tx, y, tw, fMonoBold.Height);
        Txt(g, label, tx, y, tw + S(2), fMonoBold, active ? YELLOW : Hot(r) ? TEXT : DIM);
        Hit(r, a);
    }

    // ---------- wszystkie projekty ----------

    void SortProjects(string key, bool ascFirst)
    {
        if (projectSort == key) projectAsc = !projectAsc; else { projectSort = key; projectAsc = ascFirst; }
        scroll = 0;
    }

    void DrawProjects(Graphics g, int W, int H, int y)
    {
        int pad = S(14), cw = W - 2 * pad - S(16);
        var rows = SortedProjects();
        long total = rows.Sum(p => p.Total), trash = rows.Sum(p => p.Trash);
        string head = Fmt.Plural(rows.Count, "projekt", "projekty", "projektów") + "  ·  " + Fmt.Size(total) + " łącznie  ·  " + Fmt.Size(trash) + " to śmieci do odzyskania";
        Txt(g, head, pad, y + S(2), cw, fBold, TEXT);
        y += S(28);
        // jezyki wszystkich projektow razem
        var sum = new long[Tables.Langs.Length];
        foreach (var p in rows) foreach (var l in p.Langs) sum[l.Key] += l.Value;
        var langs = sum.Select((v, k) => new KeyValuePair<int, long>(k, v)).Where(l => l.Value > 0).OrderByDescending(l => l.Value).ToList();
        LangBar(g, pad, y, cw, S(10), langs);
        y += S(16);
        long codeTotal = Math.Max(1, langs.Sum(l => l.Value));
        int lx = pad;
        foreach (var l in langs.Take(14))
        {
            string label = Tables.Langs[l.Key].Name + " " + (100.0 * l.Value / codeTotal).ToString("0.#") + "%";
            int w = Measure(g, label, fBody) + S(16);
            if (lx + w > pad + cw) break;
            Dot(g, Tables.Langs[l.Key].Color, lx + S(5), y + fBody.Height / 2 + S(1));
            Txt(g, label, lx + S(15), y, w, fBody, TEXT);
            lx += w + S(14);
        }
        y += S(30);
        // tabela
        int cDate = S(120), cTrash = S(90), cSize = S(90), cLang = (int)(cw * 0.34f);
        int xDate = pad + cw - cDate, xTrash = xDate - cTrash - S(10), xSize = xTrash - cSize - S(10), xLang = xSize - cLang - S(16);
        int nameW = xLang - pad - S(16);
        Header(g, "Projekt", pad, y, nameW, false, projectSort == "name", projectAsc, () => SortProjects("name", true));
        Txt(g, "Języki", xLang, y, cLang, fMonoBold, DIM);
        Header(g, "Rozmiar", xSize, y, cSize, true, projectSort == "size", projectAsc, () => SortProjects("size", false));
        Header(g, "Śmieci", xTrash, y, cTrash, true, projectSort == "trash", projectAsc, () => SortProjects("trash", false));
        Header(g, "Zmiana", xDate, y, cDate, true, projectSort == "date", projectAsc, () => SortProjects("date", true));
        y += S(22);
        Fill(g, FRAME, pad, y - S(2), cw, Math.Max(1, S(1)));
        viewTop = y;
        viewH = H - y - S(8);
        int detailH = S(24) * 3 + S(8);
        contentH = 0;
        foreach (var p in rows) contentH += rowH + (p.Open ? detailH + p.Items.Count * rowH : 0);
        ClampScroll();
        g.SetClip(new Rectangle(0, viewTop, W, viewH));
        int cy = viewTop - (int)scroll, i = 0;
        foreach (var p in rows)
        {
            int full = rowH + (p.Open ? detailH + p.Items.Count * rowH : 0);
            if (cy + full >= viewTop && cy < viewTop + viewH) DrawProjectRow(g, p, i, pad, cy, cw, nameW, xLang, cLang, xSize, cSize, xTrash, cTrash, xDate, cDate);
            cy += full;
            i++;
            if (cy > viewTop + viewH) break;
        }
        g.ResetClip();
    }

    void DrawProjectRow(Graphics g, Project p, int i, int pad, int y, int cw, int nameW, int xLang, int cLang, int xSize, int cSize, int xTrash, int cTrash, int xDate, int cDate)
    {
        var r = new Rectangle(pad, y, cw, rowH);
        bool hot = Hot(r) && r.IntersectsWith(new Rectangle(0, viewTop, drawW, viewH));
        if (hot) Fill(g, HOVER, r.X, r.Y, r.Width, r.Height); else if (i % 2 == 1) Fill(g, STRIPE, r.X, r.Y, r.Width, r.Height);
        int ty = y + (rowH - fBody.Height) / 2;
        Dot(g, p.Kind >= 0 && p.Kind < Tables.Langs.Length ? Tables.Langs[p.Kind].Color : DIM, pad + S(8), y + rowH / 2);
        Txt(g, p.Name, pad + S(20), ty, nameW - S(20), p.Open ? fBold : fBody, p.Open ? YELLOW : LINK);
        long total = Math.Max(1, p.Langs.Sum(l => l.Value));
        int barW = Math.Min(S(120), cLang / 3);
        LangBar(g, xLang, y + rowH / 2 - S(4), barW, S(8), p.Langs);
        string names = string.Join(" · ", p.Langs.Take(3).Select(l => Tables.Langs[l.Key].Name + " " + Math.Round(100.0 * l.Value / total) + "%"));
        Txt(g, names, xLang + barW + S(8), ty, cLang - barW - S(8), fBody, TEXT);
        TxtRight(g, Fmt.Size(p.Total), xSize + cSize, y + (rowH - fMono.Height) / 2, cSize, fMono, TEXT);
        TxtRight(g, p.Trash > 0 ? Fmt.Size(p.Trash) : "-", xTrash + cTrash, y + (rowH - fMono.Height) / 2, cTrash, fMono, p.Trash > (1L << 30) ? YELLOW : DIM);
        TxtRight(g, Fmt.Ago(p.Newest), xDate + cDate, y + (rowH - fMono.Height) / 2, cDate, fMono, DIM);
        if (y + rowH > viewTop && y < viewTop + viewH) Hit(Rectangle.Intersect(r, new Rectangle(0, viewTop, drawW, viewH)), () => p.Open = !p.Open);
        if (!p.Open) return;
        int dy = y + rowH + S(4), dx = pad + S(20);
        Txt(g, p.Path, dx, dy, cw - S(20), fMono, DIM);
        dy += S(24);
        string facts = Fmt.Plural(p.Files, "plik", "pliki", "plików") + "  ·  kod i dane " + Fmt.Size(p.Bytes) + "  ·  śmieci " + Fmt.Size(p.Trash) + (p.Git ? "  ·  repo git" : "");
        Txt(g, facts, dx, dy, cw - S(20), fBody, TEXT);
        dy += S(24);
        int lx = dx;
        lx += Link(g, "[Otwórz w Indeksie]", lx, dy, fBody, LINK, () => Send(COPY_OPEN, p.Path)) + S(18);
        lx += Link(g, "[Eksplorator]", lx, dy, fBody, LINK, () => System.Diagnostics.Process.Start("explorer.exe", "\"" + p.Path + "\"")) + S(18);
        if (p.Items.Count > 0)
            lx += Link(g, "[Wyczyść ten projekt]", lx, dy, fBody, YELLOW, () => { projectFilter = p.Path; View = 1; scroll = 0; }) + S(18);
        dy += S(24) + S(8);
        foreach (var t in p.Items.OrderByDescending(t => t.Bytes))
        {
            Dot(g, Tables.Cats[t.Cat].Color, dx + S(8), dy + rowH / 2);
            Txt(g, t.Name + "  —  " + Tables.Cats[t.Cat].Name, dx + S(20), dy + (rowH - fBody.Height) / 2, cw - S(180), fBody, TEXT);
            TxtRight(g, Fmt.Size(t.Bytes), pad + cw, dy + (rowH - fMono.Height) / 2, S(120), fMono, TEXT);
            dy += rowH;
        }
    }

    // ---------- czyszczenie dysku ----------

    void SortClean(string key, bool ascFirst)
    {
        if (cleanSort == key) cleanAsc = !cleanAsc; else { cleanSort = key; cleanAsc = ascFirst; }
        scroll = 0;
    }

    void DrawClean(Graphics g, int W, int H, int y)
    {
        int pad = S(14), cw = W - 2 * pad - S(16);
        var rows = SortedTrash();
        long all = cat.Trash.Sum(t => t.Bytes);
        var chosen = cat.Trash.Where(t => t.Selected).ToList();
        long picked = chosen.Sum(t => t.Bytes);
        // ile do odzyskania i wolne miejsce
        Txt(g, Fmt.Size(all), pad, y, S(200), fTitle, YELLOW);
        int tw = Measure(g, Fmt.Size(all), fTitle);
        Txt(g, "do odzyskania", pad + tw + S(10), y + S(6), S(150), fBody, DIM);
        string free = string.Join("   ", DriveInfo.GetDrives().Where(d => d.DriveType == DriveType.Fixed && d.IsReady).Select(d => d.Name.TrimEnd('\\') + " wolne " + Fmt.Size(d.AvailableFreeSpace)));
        int fw = Measure(g, free, fMono);
        Txt(g, free, pad + cw - fw, y + S(8), fw + S(2), fMono, TEXT);
        y += S(34);
        // presety: szybki wybór rodzaju śmieci do przejrzenia
        int qx = pad;
        qx += Measure(g, "Pokaż:", fMonoBold) + S(12);
        Txt(g, "Pokaż:", pad, y + S(2), S(80), fMonoBold, DIM);
        foreach (var pr in new[] { new[] { "all", "Wszystko" }, new[] { "build", "Biblioteki i budowanie" }, new[] { "ai", "Temp i AI" }, new[] { "old", "Dawno nieużywane" }, new[] { "logs", "Logi" }, new[] { "files", "Pliki luzem" } })
        {
            string key = pr[0], label = pr[1];
            bool on = cleanPreset == key;
            int w = Measure(g, label, on ? fBold : fBody);
            var r = new Rectangle(qx, y, w, fBody.Height);
            Txt(g, label, qx, y, w + S(2), on ? fBold : fBody, on ? YELLOW : Hot(r) ? TEXT : LINK);
            if (on) Fill(g, YELLOW, qx, y + fBody.Height + S(1), w, Math.Max(1, S(1)));
            Hit(r, () => { cleanPreset = key; scroll = 0; });
            qx += w + S(18);
        }
        y += S(28);
        // kategorie: pasek udzialow + wlaczanie/wylaczanie
        var byCat = VisibleTrash().GroupBy(t => t.Cat).Select(c => new KeyValuePair<int, long>(c.Key, c.Sum(t => t.Bytes))).OrderByDescending(c => c.Value).ToList();
        int bx = pad;
        long sum = Math.Max(1, byCat.Sum(c => c.Value));
        foreach (var c in byCat)
        {
            int w = (int)(cw * (double)c.Value / sum);
            if (w > 0) Fill(g, hiddenCats.Contains(c.Key) ? BAR : Tables.Cats[c.Key].Color, bx, y, w, S(10));
            bx += w;
        }
        y += S(16);
        int cx = pad;
        foreach (var c in byCat)
        {
            int key = c.Key;
            bool off = hiddenCats.Contains(key);
            string label = Tables.Cats[key].Name + " " + Fmt.Size(c.Value);
            int w = Measure(g, label, fBody) + S(16);
            if (cx + w > pad + cw) { cx = pad; y += S(24); }
            Dot(g, off ? BAR : Tables.Cats[key].Color, cx + S(5), y + fBody.Height / 2 + S(1));
            var r = new Rectangle(cx, y, w, fBody.Height);
            Txt(g, label, cx + S(15), y, w, fBody, off ? DIM : Hot(r) ? LINK : TEXT);
            Hit(r, () => { if (!hiddenCats.Remove(key)) hiddenCats.Add(key); scroll = 0; });
            cx += w + S(14);
        }
        y += S(30);
        // akcje
        int ax = pad;
        ax += Link(g, "[Zaznacz pewne starsze niż 30 dni]", ax, y, fBody, LINK, SelectSafeOld) + S(18);
        ax += Link(g, "[Odznacz wszystko]", ax, y, fBody, LINK, () => { foreach (var t in cat.Trash) t.Selected = false; }) + S(18);
        if (projectFilter != null)
            ax += Link(g, "[Pokaż wszystkie projekty]", ax, y, fBody, LINK, () => { projectFilter = null; scroll = 0; }) + S(18);
        string info = chosen.Count > 0 ? "Zaznaczone: " + Fmt.Size(picked) + " (" + Fmt.Plural(chosen.Count, "pozycja", "pozycje", "pozycji") + ")" : "Nic nie jest zaznaczone";
        string del = "[Usuń zaznaczone]";
        int dw = Measure(g, del, fBold), iw = Measure(g, info, fBody);
        if (chosen.Count > 0 && remover == null) Link(g, del, pad + cw - dw, y, fBold, ERR, () => confirm = true);
        else Txt(g, del, pad + cw - dw, y, dw + S(2), fBold, FRAME);
        Txt(g, info, pad + cw - dw - S(16) - iw, y, iw + S(2), fBody, chosen.Count > 0 ? TEXT : DIM);
        y += S(26);
        if (status.Length > 0) { Txt(g, status, pad, y, cw, fBody, GREEN); y += S(24); }
        // tabela
        int cSize = S(90), cAge = S(120), cSafe = S(90), cCat = S(190);
        int xSize = pad + cw - cSize, xAge = xSize - cAge - S(10), xSafe = xAge - cSafe - S(10), xCat = xSafe - cCat - S(10);
        int xName = pad + S(52);
        Txt(g, "Folder", xName, y, xCat - xName, fMonoBold, DIM);
        Txt(g, "Rodzaj", xCat, y, cCat, fMonoBold, DIM);
        Txt(g, "Ocena", xSafe, y, cSafe, fMonoBold, DIM);
        Header(g, "Wiek", xAge, y, cAge, true, cleanSort == "age", cleanAsc, () => SortClean("age", true));
        Header(g, "Rozmiar", xSize, y, cSize, true, cleanSort == "size", cleanAsc, () => SortClean("size", false));
        y += S(22);
        Fill(g, FRAME, pad, y - S(2), cw, Math.Max(1, S(1)));
        viewTop = y;
        viewH = H - y - S(8);
        contentH = 0;
        foreach (var t in rows) contentH += rowH * (t.Open ? 2 + t.Top.Count : 1);
        ClampScroll();
        var clip = new Rectangle(0, viewTop, W, viewH);
        g.SetClip(clip);
        int cy = viewTop - (int)scroll, i = 0;
        uint now = Fmt.Now();
        foreach (var t in rows)
        {
            int full = rowH * (t.Open ? 2 + t.Top.Count : 1);
            if (cy + full >= viewTop && cy < viewTop + viewH) DrawTrashRow(g, t, i, pad, cy, cw, xName, xCat, cCat, xSafe, cSafe, xAge, cAge, xSize, cSize, now, clip);
            cy += full;
            i++;
            if (cy > viewTop + viewH) break;
        }
        g.ResetClip();
    }

    void DrawTrashRow(Graphics g, TrashEntry t, int i, int pad, int y, int cw, int xName, int xCat, int cCat, int xSafe, int cSafe, int xAge, int cAge, int xSize, int cSize, uint now, Rectangle clip)
    {
        var row = new Rectangle(pad, y, cw, rowH);
        if (Hot(row) && clip.Contains(mouse)) Fill(g, HOVER, row.X, row.Y, row.Width, row.Height); else if (i % 2 == 1) Fill(g, STRIPE, row.X, row.Y, row.Width, row.Height);
        int ty = y + (rowH - fBody.Height) / 2, my = y + (rowH - fMono.Height) / 2;
        // pole wyboru
        int box = S(14), bxx = pad + S(4), byy = y + (rowH - box) / 2, p = Math.Max(1, S(1));
        Fill(g, t.Selected ? YELLOW : FRAME, bxx, byy, box, box);
        if (!t.Selected) Fill(g, BG, bxx + p, byy + p, box - 2 * p, box - 2 * p);
        var boxHit = Rectangle.Intersect(new Rectangle(pad, y, S(24), rowH), clip);
        if (!boxHit.IsEmpty) Hit(boxHit, () => t.Selected = !t.Selected);
        // [+] pokazuje pliki w srodku
        var openRect = new Rectangle(pad + S(24), y, S(26), rowH);
        Txt(g, t.Open ? "[-]" : "[+]", openRect.X, my, openRect.Width, fMono, t.Top.Count > 0 ? (Hot(openRect) ? TEXT : DIM) : FRAME);
        // nazwa + projekt albo folder nadrzedny
        string parent = t.Project.Length > 0 ? Path.GetFileName(t.Project) : Path.GetDirectoryName(t.Path.TrimEnd('*', '\\')) ?? "";
        int nameW = Measure(g, t.Name, fBody);
        int room = xCat - xName - S(10);
        Txt(g, t.Name, xName, ty, Math.Min(nameW + S(2), room), fBody, LINK);
        if (nameW + S(14) < room) Txt(g, parent, xName + nameW + S(10), my, room - nameW - S(10), fMono, DIM);
        var c = Tables.Cats[t.Cat];
        Dot(g, c.Color, xCat + S(5), y + rowH / 2);
        Txt(g, c.Name, xCat + S(15), ty, cCat - S(15), fBody, TEXT);
        bool fresh = t.Newest != 0 && now - t.Newest < 1440;
        Txt(g, fresh ? "w użyciu?" : t.Safety == 1 ? "pewne" : "sprawdź", xSafe, ty, cSafe, fBody, fresh ? ERR : t.Safety == 1 ? GREEN : YELLOW);
        TxtRight(g, Fmt.Ago(t.Newest), xAge + cAge, my, cAge, fMono, DIM);
        TxtRight(g, Fmt.Size(t.Bytes), xSize + cSize, my, cSize, fMono, TEXT);
        var rest = Rectangle.Intersect(new Rectangle(pad + S(24), y, cw - S(24), rowH), clip);
        if (!rest.IsEmpty && t.Top.Count > 0) Hit(rest, () => t.Open = !t.Open);
        if (!t.Open) return;
        int dy = y + rowH;
        Txt(g, t.Path, xName, dy + (rowH - fMono.Height) / 2, xSize - xName, fMono, DIM);
        TxtRight(g, "przywrócenie: " + c.Restore, xSize + cSize, dy + (rowH - fMono.Height) / 2, S(320), fMono, DIM);
        dy += rowH;
        foreach (var f in t.Top)
        {
            Txt(g, f.Rel, xName + S(16), dy + (rowH - fMono.Height) / 2, xAge - xName - S(26), fMono, TEXT);
            TxtRight(g, Fmt.Ago(f.Time), xAge + cAge, dy + (rowH - fMono.Height) / 2, cAge, fMono, DIM);
            TxtRight(g, Fmt.Size(f.Bytes), xSize + cSize, dy + (rowH - fMono.Height) / 2, cSize, fMono, TEXT);
            dy += rowH;
        }
    }

    void DrawConfirm(Graphics g, int W, int H)
    {
        // okno przechwytuje klikniecia: nic pod spodem nie moze sie wykonac
        hits.Clear();
        var chosen = cat.Trash.Where(t => t.Selected).ToList();
        int dw = S(640), dh = S(232), x = (W - dw) / 2, y = (H - dh) / 2, p = Math.Max(1, S(1));
        Fill(g, FRAME, x, y, dw, dh);
        Fill(g, BG, x + p, y + p, dw - 2 * p, dh - 2 * p);
        int tx = x + S(18), ty = y + S(14), tw = dw - S(36);
        Txt(g, "Usunąć trwale?", tx, ty, tw, fTitle, ERR);
        ty += S(38);
        Txt(g, Fmt.Plural(chosen.Count, "pozycja", "pozycje", "pozycji") + ", razem " + Fmt.Size(chosen.Sum(t => t.Bytes)) + ".", tx, ty, tw, fBold, TEXT);
        ty += S(26);
        Txt(g, "Pliki nie trafią do Kosza i nie da się ich cofnąć.", tx, ty, tw, fBody, TEXT);
        ty += S(26);
        int check = chosen.Count(t => t.Safety != 1);
        uint now = Fmt.Now();
        int fresh = chosen.Count(t => t.Newest != 0 && now - t.Newest < 1440);
        if (check > 0) { Txt(g, "Wśród nich: " + Fmt.Plural(check, "pozycja", "pozycje", "pozycji") + " z oceną „sprawdź”.", tx, ty, tw, fBody, YELLOW); ty += S(24); }
        if (fresh > 0) { Txt(g, Fmt.Plural(fresh, "pozycja zmieniona", "pozycje zmienione", "pozycji zmienionych") + " w ciągu ostatniej doby.", tx, ty, tw, fBody, ERR); }
        int by = y + dh - S(36);
        int w1 = Measure(g, "[Usuń trwale]", fBold);
        bool was = confirm;
        confirm = false;  // Link sprawdza najechanie tylko poza oknem potwierdzenia
        Link(g, "[Usuń trwale]", tx, by, fBold, ERR, StartRemoval);
        Link(g, "[Anuluj]", tx + w1 + S(30), by, fBody, LINK, () => confirm = false);
        confirm = was;
        // klik obok okna nic nie robi, ale nie moze trafic w liste
        hits.Insert(0, new HitBox { R = new Rectangle(0, 0, W, H), Do = () => { } });
    }
}
