// Indeks - silnik katalogu (C++), punkt wejscia.
//   indeks-katalog.exe --pelny              przeszukuje caly komputer i zapisuje katalog
//   indeks-katalog.exe --odswiez LISTA      przelicza tylko foldery z pliku LISTA i scala
//   indeks-katalog.exe --usun LISTA         usuwa pozycje z LISTA (tylko te obecne w katalogu)
//   --plik SCIEZKA                          inny plik katalogu (domyslnie %LOCALAPPDATA%\Indeks)
#include "katalog.h"

#include <algorithm>
#include <chrono>
#include <fstream>
#include <set>
#include <sstream>

bool read_catalog(const std::wstring& file, Catalog& out) {
    std::ifstream f(file, std::ios::binary);
    if (!f) return false;
    std::string line;
    while (std::getline(f, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        if (line.empty()) continue;
        switch (line[0]) {
            case 'V': out.header = line; break;
            case 'F': if (!out.records.empty()) out.records.back().files.push_back(line); break;
            case 'P': case 'T': out.records.push_back(Record{line, {}}); break;
            default: break;
        }
    }
    return !out.header.empty();
}

void write_text(const std::wstring& file, const std::string& text) {
    std::wstring tmp = file + L".tmp";
    {
        std::ofstream f(tmp, std::ios::binary | std::ios::trunc);
        if (!f) return;
        f << text;
    }
    MoveFileExW(tmp.c_str(), file.c_str(), MOVEFILE_REPLACE_EXISTING);
}

bool write_catalog(const std::wstring& file, const Catalog& c) {
    std::wstring tmp = file + L".tmp";
    {
        std::ofstream f(tmp, std::ios::binary | std::ios::trunc);
        if (!f) return false;
        f << c.header << '\n';
        for (const auto& r : c.records) {
            f << r.line << '\n';
            for (const auto& l : r.files) f << l << '\n';
        }
        if (!f.good()) return false;
    }
    // zapis przez podmiane: czytelnik nigdy nie zobaczy polowy pliku
    return MoveFileExW(tmp.c_str(), file.c_str(), MOVEFILE_REPLACE_EXISTING) != 0;
}

static std::vector<std::wstring> read_list(const std::wstring& file) {
    std::vector<std::wstring> v;
    std::ifstream f(file, std::ios::binary);
    std::string line;
    while (std::getline(f, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        if (line.size() >= 3 && (unsigned char)line[0] == 0xEF) line.erase(0, 3);
        if (!line.empty()) v.push_back(wide(line));
    }
    return v;
}

static std::string make_header(uint32_t scan_min, uint64_t ms, uint32_t dirs, uint32_t changed) {
    std::ostringstream h;
    h << "V\t1\t" << scan_min << '\t' << ms << '\t' << dirs << '\t' << changed;
    return h.str();
}

// ---------- usuwanie ----------

struct Removal {
    uint64_t freed = 0;
    uint32_t failed = 0;
    uint64_t total = 0;
    uint64_t base = 0;
    std::wstring progress;
    std::string current;
    DWORD last_write = 0;
};

static void report(Removal& r) {
    DWORD now = GetTickCount();
    if (now - r.last_write < 400) return;
    r.last_write = now;
    std::ostringstream s;
    s << "usun\t" << r.base + r.freed << '\t' << r.total << '\t' << r.current;
    write_text(r.progress, s.str());
}

static void remove_file(const std::wstring& p, uint64_t size, DWORD attrs, Removal& r) {
    if (attrs & FILE_ATTRIBUTE_READONLY) SetFileAttributesW(p.c_str(), attrs & ~FILE_ATTRIBUTE_READONLY);
    if (DeleteFileW(p.c_str())) r.freed += size; else r.failed++;
}

// Dowiazania (junction, symlink) usuwane sa jako same dowiazania: pnpm i inne narzedzia
// wskazuja nimi foldery spoza usuwanej pozycji, ktorych nie wolno ruszyc.
static void remove_tree(const std::wstring& dir, bool only_files, Removal& r) {
    WIN32_FIND_DATAW fd;
    HANDLE h = FindFirstFileExW((dir + L"\\*").c_str(), FindExInfoBasic, &fd, FindExSearchNameMatch, nullptr, FIND_FIRST_EX_LARGE_FETCH);
    if (h == INVALID_HANDLE_VALUE) { r.failed++; return; }
    do {
        if (!wcscmp(fd.cFileName, L".") || !wcscmp(fd.cFileName, L"..")) continue;
        std::wstring p = dir + L"\\" + fd.cFileName;
        bool is_dir = (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
        bool link = (fd.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0;
        if (is_dir && only_files) continue;
        if (is_dir) {
            if (fd.dwFileAttributes & FILE_ATTRIBUTE_READONLY) SetFileAttributesW(p.c_str(), fd.dwFileAttributes & ~FILE_ATTRIBUTE_READONLY);
            if (!link) remove_tree(p, false, r);
            if (!RemoveDirectoryW(p.c_str())) r.failed++;
        } else {
            uint64_t size = link ? 0 : (((uint64_t)fd.nFileSizeHigh << 32) | fd.nFileSizeLow);
            remove_file(p, (size + 4095) & ~4095ull, fd.dwFileAttributes, r);
        }
        report(r);
    } while (FindNextFileW(h, &fd));
    FindClose(h);
}

// Ile zostalo po usuwaniu (zablokowane pliki): (bajty, pliki).
static void measure(const std::wstring& dir, bool only_files, uint64_t& bytes, uint32_t& files) {
    WIN32_FIND_DATAW fd;
    HANDLE h = FindFirstFileExW((dir + L"\\*").c_str(), FindExInfoBasic, &fd, FindExSearchNameMatch, nullptr, FIND_FIRST_EX_LARGE_FETCH);
    if (h == INVALID_HANDLE_VALUE) return;
    do {
        if (!wcscmp(fd.cFileName, L".") || !wcscmp(fd.cFileName, L"..")) continue;
        if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) {
            if (!only_files && !(fd.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT)) measure(dir + L"\\" + fd.cFileName, false, bytes, files);
        } else {
            bytes += ((((uint64_t)fd.nFileSizeHigh << 32) | fd.nFileSizeLow) + 4095) & ~4095ull;
            files++;
        }
    } while (FindNextFileW(h, &fd));
    FindClose(h);
}

static int run_remove(const std::wstring& file, const std::wstring& list_file) {
    Catalog c;
    if (!read_catalog(file, c)) return 2;
    std::set<std::wstring> wanted;
    for (const auto& p : read_list(list_file)) wanted.insert(lower(p));
    Removal r;
    r.progress = file + L".postep";
    // usuwamy wylacznie pozycje z katalogu; sciezka spoza niego jest ignorowana
    std::vector<size_t> todo;
    for (size_t i = 0; i < c.records.size(); i++) {
        const Record& rec = c.records[i];
        if (rec.type() != 'T' || !wanted.count(lower(rec.path()))) continue;
        todo.push_back(i);
        auto f = split_tabs(rec.line);
        if (f.size() > 4) r.total += _strtoui64(f[4].c_str(), nullptr, 10);
    }
    std::vector<bool> drop(c.records.size(), false);
    for (size_t i : todo) {
        Record& rec = c.records[i];
        std::wstring path = rec.path();
        if (path.size() < 6 || path.find(L'\\') == std::wstring::npos) continue;
        r.current = utf8(path);
        r.base += r.freed;
        r.freed = 0;
        bool loose = path.size() > 2 && path.compare(path.size() - 2, 2, L"\\*") == 0;
        std::wstring target = longp(loose ? path.substr(0, path.size() - 2) : path);
        DWORD attrs = GetFileAttributesW(target.c_str());
        if (attrs == INVALID_FILE_ATTRIBUTES) { drop[i] = true; continue; }
        uint64_t left = 0;
        uint32_t left_files = 0;
        if (!(attrs & FILE_ATTRIBUTE_DIRECTORY)) {
            WIN32_FILE_ATTRIBUTE_DATA d{};
            GetFileAttributesExW(target.c_str(), GetFileExInfoStandard, &d);
            uint64_t size = ((uint64_t)d.nFileSizeHigh << 32) | d.nFileSizeLow;
            remove_file(target, size, attrs, r);
            if (GetFileAttributesW(target.c_str()) != INVALID_FILE_ATTRIBUTES) { left = size; left_files = 1; }
        } else if (attrs & FILE_ATTRIBUTE_REPARSE_POINT) {
            if (!RemoveDirectoryW(target.c_str())) r.failed++;
        } else {
            remove_tree(target, loose, r);
            if (!loose) RemoveDirectoryW(target.c_str());  // folder z zablokowanymi plikami zostaje
            if (GetFileAttributesW(target.c_str()) != INVALID_FILE_ATTRIBUTES) measure(target, loose, left, left_files);
        }
        report(r);
        if (left == 0) {
            drop[i] = true;
        } else {
            auto f = split_tabs(rec.line);
            if (f.size() >= 8) {
                f[4] = std::to_string(left);
                f[5] = std::to_string(left_files);
                std::string joined = f[0];
                for (size_t k = 1; k < f.size(); k++) joined += "\t" + f[k];
                rec.line = joined;
                rec.files.clear();
            }
        }
    }
    r.base += r.freed;
    std::vector<Record> kept;
    kept.reserve(c.records.size());
    for (size_t i = 0; i < c.records.size(); i++)
        if (!drop[i]) kept.push_back(std::move(c.records[i]));
    c.records.swap(kept);
    write_catalog(file, c);
    std::ostringstream res;
    res << r.base << '\t' << r.failed << '\t' << todo.size();
    write_text(file + L".wynik", res.str());
    DeleteFileW(r.progress.c_str());
    return 0;
}

// ---------- skan ----------

static int run_full(const std::wstring& file) {
    auto t0 = std::chrono::steady_clock::now();
    Catalog c;
    uint32_t dirs = 0;
    std::wstring progress = file + L".postep";
    scan({}, true, progress, c.records, dirs);
    uint64_t ms = (uint64_t)std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - t0).count();
    uint32_t now = now_minutes();
    c.header = make_header(now, ms, dirs, now);
    bool ok = write_catalog(file, c);
    DeleteFileW(progress.c_str());
    return ok ? 0 : 3;
}

static int run_refresh(const std::wstring& file, const std::wstring& list_file) {
    Catalog c;
    if (!read_catalog(file, c)) return 2;  // bez pelnego katalogu nie ma czego uzupelniac
    // folder wewnatrz znanego projektu: przeliczamy caly projekt, zeby nie powstal zagniezdzony
    std::vector<std::wstring> roots, low;
    for (const auto& given : read_list(list_file)) {
        std::wstring p = given, lp = lower(p);
        for (const auto& r : c.records) {
            if (r.type() != 'P') continue;
            std::wstring rp = r.path(), lr = lower(rp);
            if (under(lp, lr)) { p = rp; lp = lr; break; }
        }
        if (std::find(low.begin(), low.end(), lp) == low.end()) { roots.push_back(p); low.push_back(lp); }
    }
    if (roots.empty()) return 0;
    std::vector<Record> kept;
    for (auto& r : c.records) {
        std::wstring lp = lower(r.path());
        bool inside = std::any_of(low.begin(), low.end(), [&](const std::wstring& root) { return under(lp, root); });
        if (!inside) kept.push_back(std::move(r));
    }
    std::vector<Record> fresh;
    uint32_t dirs = 0;
    std::vector<std::wstring> existing;
    for (const auto& p : roots)
        if (GetFileAttributesW(longp(p).c_str()) != INVALID_FILE_ATTRIBUTES) existing.push_back(p);
    if (!existing.empty()) scan(existing, false, L"", fresh, dirs);
    for (auto& r : fresh) kept.push_back(std::move(r));
    c.records.swap(kept);
    auto h = split_tabs(c.header);
    if (h.size() >= 6) {
        h[5] = std::to_string(now_minutes());
        std::string joined = h[0];
        for (size_t k = 1; k < h.size(); k++) joined += "\t" + h[k];
        c.header = joined;
    }
    return write_catalog(file, c) ? 0 : 3;
}

int wmain(int argc, wchar_t** argv) {
    std::wstring mode = argc > 1 ? argv[1] : L"", arg, file;
    for (int i = 2; i < argc; i++) {
        if (!wcscmp(argv[i], L"--plik") && i + 1 < argc) file = argv[++i];
        else if (arg.empty()) arg = argv[i];
    }
    if (file.empty()) {
        std::wstring dir = env(L"LOCALAPPDATA") + L"\\Indeks";
        CreateDirectoryW(dir.c_str(), nullptr);
        file = dir + L"\\katalog.tsv";
    }
    // jeden silnik naraz: pelny skan nie czeka, reszta poczeka na swoja kolej
    HANDLE m = CreateMutexW(nullptr, FALSE, L"Local\\Indeks.Katalog");
    DWORD wait = WaitForSingleObject(m, mode == L"--pelny" ? 0 : 600000);
    if (wait != WAIT_OBJECT_0 && wait != WAIT_ABANDONED) return 4;
    int rc = 1;
    if (mode == L"--pelny") {
        SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
        rc = run_full(file);
    } else if (mode == L"--odswiez") {
        // odswiezanie w tle: najnizszy priorytet procesora i dysku
        SetPriorityClass(GetCurrentProcess(), PROCESS_MODE_BACKGROUND_BEGIN);
        rc = run_refresh(file, arg);
    } else if (mode == L"--usun") {
        rc = run_remove(file, arg);
    }
    ReleaseMutex(m);
    return rc;
}
