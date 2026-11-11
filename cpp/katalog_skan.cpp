// Indeks - silnik katalogu (C++): przeszukiwanie dysku.
// Kilka watkow bierze foldery ze wspolnego stosu. Wezel w pamieci dostaja tylko foldery
// zwykle; zawartosc smieci (node_modules itp.) i folderow ukrytych jest tylko sumowana,
// wiec miliony plikow nie zajmuja pamieci.
#include "katalog.h"

#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <map>
#include <mutex>
#include <sstream>
#include <thread>
#include <unordered_map>
#include <unordered_set>

namespace {

// Bity znacznikow projektu (zgodne z src/profiles.rs do bitu 15).
constexpr uint32_t M_PACKAGE = 1u << 0, M_CARGO = 1u << 1, M_CMAKE = 1u << 2, M_SLN = 1u << 3, M_VCXPROJ = 1u << 4,
    M_CSPROJ = 1u << 5, M_PYTHON = 1u << 6, M_GO = 1u << 7, M_JAVA = 1u << 8, M_PHP = 1u << 9, M_RUBY = 1u << 10,
    M_DART = 1u << 11, M_GIT = 1u << 12, M_TSCONFIG = 1u << 13, M_HTML = 1u << 14,
    M_NODELOCK = 1u << 16, M_COMPOSERLOCK = 1u << 17, M_PINE = 1u << 18, M_MQL = 1u << 19, M_JS = 1u << 20, M_KT = 1u << 21;

constexpr int LANGS = 32, KIND_GIT = 100;

// Identyfikatory zgodne z tablica LANGS w src/profiles.rs.
const std::unordered_map<std::wstring, uint8_t>& ext_table() {
    static const std::unordered_map<std::wstring, uint8_t> t = {
        {L"js", 0}, {L"mjs", 0}, {L"cjs", 0}, {L"jsx", 0}, {L"ts", 1}, {L"tsx", 1}, {L"mts", 1}, {L"cts", 1}, {L"rs", 2},
        {L"cpp", 3}, {L"cc", 3}, {L"cxx", 3}, {L"hpp", 3}, {L"hh", 3}, {L"hxx", 3}, {L"inl", 3}, {L"c", 4}, {L"h", 4}, {L"cs", 5},
        {L"py", 6}, {L"pyw", 6}, {L"ipynb", 6}, {L"go", 7}, {L"java", 8}, {L"kt", 9}, {L"kts", 9}, {L"php", 10}, {L"rb", 11},
        {L"html", 12}, {L"htm", 12}, {L"css", 13}, {L"scss", 13}, {L"sass", 13}, {L"less", 13},
        {L"sh", 14}, {L"bash", 14}, {L"ps1", 14}, {L"psm1", 14}, {L"bat", 14}, {L"cmd", 14}, {L"lua", 15}, {L"dart", 16}, {L"swift", 17},
        {L"mq4", 18}, {L"mq5", 18}, {L"mqh", 18}, {L"pine", 19}, {L"vue", 20}, {L"svelte", 21}, {L"sql", 22}, {L"asm", 23}, {L"s", 23},
        {L"zig", 24}, {L"r", 25}, {L"scala", 26}, {L"hs", 27}, {L"jl", 28}, {L"wat", 29},
        {L"glsl", 30}, {L"hlsl", 30}, {L"wgsl", 30}, {L"frag", 30}, {L"vert", 30},
    };
    return t;
}

bool is_code_lang(int l) { return l != 12 && l != 13 && l != 14 && l != 22; }

bool ends_with(const std::wstring& s, const wchar_t* suffix) {
    size_t n = wcslen(suffix);
    return s.size() >= n && s.compare(s.size() - n, n, suffix) == 0;
}

uint32_t marker_of(const std::wstring& n) {
    if (n == L"package.json") return M_PACKAGE;
    if (n == L"cargo.toml") return M_CARGO;
    if (n == L"cmakelists.txt") return M_CMAKE;
    if (ends_with(n, L".sln")) return M_SLN;
    if (ends_with(n, L".vcxproj")) return M_VCXPROJ;
    if (ends_with(n, L".csproj") || ends_with(n, L".fsproj") || ends_with(n, L".vbproj")) return M_CSPROJ;
    if (n == L"pyproject.toml" || n == L"requirements.txt" || n == L"setup.py") return M_PYTHON;
    if (n == L"go.mod") return M_GO;
    if (n == L"pom.xml" || n == L"build.gradle" || n == L"build.gradle.kts") return M_JAVA;
    if (n == L"composer.json") return M_PHP;
    if (n == L"gemfile") return M_RUBY;
    if (n == L"pubspec.yaml") return M_DART;
    if (n == L"tsconfig.json") return M_TSCONFIG;
    if (n == L"index.html") return M_HTML;
    if (n == L"package-lock.json" || n == L"yarn.lock" || n == L"pnpm-lock.yaml" || n == L"bun.lockb" || n == L"bun.lock") return M_NODELOCK;
    if (n == L"composer.lock") return M_COMPOSERLOCK;
    return 0;
}

struct TopFile { std::wstring rel; uint64_t bytes; uint32_t mt; };

struct Node;

struct Trash {
    std::wstring path;
    int cat, safety;
    Node* owner;
    std::mutex mu;
    uint64_t bytes = 0;
    uint32_t files = 0, newest = 0;
    std::vector<TopFile> top;
};

struct Node {
    Node* parent = nullptr;
    Node* proj = nullptr;
    std::wstring name;  // korzen: pelna sciezka
    uint64_t bytes = 0;
    std::atomic<uint64_t> extra{0};  // foldery ukryte w projekcie (.git, .github...)
    uint32_t files = 0, newest = 0, markers = 0;
    int kind = -1;
    bool vendored = false;
    std::vector<std::pair<uint8_t, uint64_t>> langs;
};

struct Work {
    std::wstring path;
    Node* node;
    Node* owner;
    Trash* trash;
    bool zone, vendored, temp_root;
};

struct Explicit { int cat, safety; bool children; };

struct Scanner {
    std::mutex mu;
    std::condition_variable cv;
    std::vector<Work> stack;
    size_t busy = 0;
    bool done = false;
    std::vector<Node*> nodes;
    std::vector<Trash*> trashes;
    std::map<std::wstring, Explicit> explicit_paths;
    std::unordered_set<std::wstring> not_projects;
    std::vector<Record> logs;
    std::atomic<uint64_t> seen_files{0}, seen_bytes{0};
    std::atomic<uint32_t> seen_dirs{0};
    uint32_t now = now_minutes();
};

constexpr size_t TOP_FILES = 40;
constexpr uint64_t TOP_MIN = 64 * 1024, LOG_MIN = 1024 * 1024;

void add_top(Trash* t, std::vector<TopFile>& local) {
    for (auto& f : local) t->top.push_back(std::move(f));
    if (t->top.size() > TOP_FILES * 2) {
        std::nth_element(t->top.begin(), t->top.begin() + TOP_FILES, t->top.end(), [](const TopFile& a, const TopFile& b) { return a.bytes > b.bytes; });
        t->top.resize(TOP_FILES);
    }
}

// Znane miejsca, gdzie narzedzia AI i menedzery pakietow zostawiaja pliki robocze.
// Historia rozmow, pamiec, ustawienia i klucze (.claude\projects, .codex\sessions...) nie sa tu wymienione celowo.
void known_places(Scanner& s) {
    std::wstring home = lower(env(L"USERPROFILE")), local = lower(env(L"LOCALAPPDATA")), roam = lower(env(L"APPDATA"));
    auto put = [&](const std::wstring& p, int cat, int safety, bool children = false) { s.explicit_paths[p] = Explicit{cat, safety, children}; };
    for (const wchar_t* n : {L"cache", L"paste-cache", L"shell-snapshots", L"session-env"}) put(home + L"\\.claude\\" + n, CAT_AI, SAFE);
    for (const wchar_t* n : {L"downloads", L"file-history"}) put(home + L"\\.claude\\" + n, CAT_AI, CHECK);
    for (const wchar_t* n : {L".tmp", L"tmp", L"cache", L"log", L"computer-use-turn-ended"}) put(home + L"\\.codex\\" + n, CAT_AI, SAFE);
    for (const wchar_t* n : {L"node_repl", L".sandbox", L".sandbox-bin", L"browser", L"process_manager", L"computer-use"}) put(home + L"\\.codex\\" + n, CAT_AI, CHECK);
    put(local + L"\\claude-cli-nodejs", CAT_AI, SAFE);
    put(home + L"\\.gemini\\tmp", CAT_AI, SAFE);
    put(home + L"\\.cursor-sandbox", CAT_AI, CHECK);
    put(local + L"\\temp", CAT_TEMP, SAFE, true);
    put(L"c:\\windows\\temp", CAT_TEMP, SAFE, true);
    put(local + L"\\temp\\claude", CAT_AI, SAFE, true);
    put(L"c:\\windows\\temp\\claude", CAT_AI, SAFE, true);
    for (const std::wstring& p : {local + L"\\npm-cache", roam + L"\\npm-cache", home + L"\\.npm\\_cacache", home + L"\\.npm\\_npx", local + L"\\pnpm-cache", local + L"\\yarn\\cache",
                                  local + L"\\pip\\cache", local + L"\\uv\\cache", home + L"\\.cargo\\registry\\cache", home + L"\\.cargo\\registry\\src", home + L"\\.gradle\\caches",
                                  local + L"\\node-gyp", local + L"\\electron\\cache", local + L"\\electron-builder\\cache", home + L"\\.pkg-cache", home + L"\\.nexe"})
        put(p, CAT_PKG, SAFE);
    for (const std::wstring& p : {local + L"\\ms-playwright", local + L"\\ms-playwright-go", local + L"\\ms-playwright-mcp", home + L"\\.cache\\puppeteer", home + L"\\.nuget\\packages", home + L"\\.m2\\repository", local + L"\\deno"})
        put(p, CAT_PKG, CHECK);
    for (const std::wstring& p : {home, home + L"\\desktop", home + L"\\documents", home + L"\\downloads", home + L"\\onedrive", home + L"\\onedrive\\desktop", home + L"\\source", home + L"\\source\\repos"})
        s.not_projects.insert(p);
}

bool has_file(const std::wstring& dir, const wchar_t* name) {
    return GetFileAttributesW(longp(dir + L"\\" + name).c_str()) != INVALID_FILE_ATTRIBUTES;
}

// Czy folder `name` (dziecko folderu ze znacznikami `m`) to odtwarzalne smieci projektu.
bool classify(uint32_t m, const std::wstring& name, const std::wstring& parent_lower, const std::wstring& full, int& cat, int& safety) {
    auto set = [&](int c, int s) { cat = c; safety = s; return true; };
    if (name == L"node_modules") {
        // spakowane programy Electron maja node_modules obok package.json - to nie projekt
        if (parent_lower.find(L"\\resources\\app") != std::wstring::npos) return false;
        return set(CAT_NODE, (m & M_PACKAGE) && (m & M_NODELOCK) ? SAFE : CHECK);
    }
    if (name == L"target" && (m & M_CARGO)) return set(CAT_RUST, SAFE);
    if (name == L"target" && (m & M_JAVA)) return set(CAT_JVM, SAFE);
    if ((name == L"bin" || name == L"obj") && (m & M_CSPROJ)) return set(CAT_DOTNET, SAFE);
    if ((name == L"debug" || name == L"release" || name == L"x64" || name == L"x86" || name == L"arm64" || name == L"ipch") && (m & (M_VCXPROJ | M_SLN | M_CSPROJ))) return set(CAT_NATIVE, SAFE);
    if (name == L".vs" || name.rfind(L"cmake-build-", 0) == 0) return set(CAT_NATIVE, SAFE);
    if (name == L"cmakefiles" && (m & M_CMAKE)) return set(CAT_NATIVE, SAFE);
    if (name == L"build" && (m & M_CMAKE)) return set(CAT_NATIVE, has_file(full, L"CMakeCache.txt") ? SAFE : CHECK);
    if (name == L"__pycache__" || name == L".pytest_cache" || name == L".mypy_cache" || name == L".ruff_cache" || name == L".tox") return set(CAT_PYTHON, SAFE);
    if ((name == L".venv" || name == L"venv" || name == L"env") && has_file(full, L"pyvenv.cfg")) return set(CAT_PYTHON, (m & M_PYTHON) ? SAFE : CHECK);
    if (m & M_PACKAGE) {
        if (name == L".next" || name == L".nuxt" || name == L".output" || name == L".turbo" || name == L".parcel-cache" || name == L".svelte-kit" || name == L".angular" || name == L".vite" || name == L".cache") return set(CAT_WEB, SAFE);
        if (name == L"dist" || name == L"build" || name == L"out" || name == L"coverage") return set(CAT_WEB, CHECK);
    }
    if (m & M_JAVA) {
        if (name == L".gradle" || name == L"build") return set(CAT_JVM, SAFE);
    }
    if (m & M_DART) {
        if (name == L".dart_tool" || name == L"build") return set(CAT_JVM, SAFE);
    }
    if (name == L"vendor" && (m & M_COMPOSERLOCK)) return set(CAT_JVM, SAFE);
    return false;
}

bool skip_anywhere(const std::wstring& n) {
    return n == L"$recycle.bin" || n == L"system volume information" || n == L"steamapps" || n == L"windowsapps" || n == L"wpsystem";
}

bool skip_at_drive(const std::wstring& n) {
    return n == L"windows" || n == L"windows.old" || n == L"recovery" || n == L"perflogs" || n == L"$winreagent" || n == L"config.msi" || n == L"msocache" ||
           n == L"programdata" || n == L"program files" || n == L"program files (x86)" || n == L"$getcurrent" || n == L"$sysreset" || n == L"$windows.~bt" || n == L"$windows.~ws";
}

bool zone_name(const std::wstring& n) { return n[0] == L'.' || n[0] == L'$' || n == L"appdata"; }

bool vendored_name(const std::wstring& n) {
    return n == L"vendor" || n == L"third_party" || n == L"thirdparty" || n == L"3rdparty" || n == L"external" || n == L"externals" || n == L"deps" || n == L"bower_components";
}

Trash* new_trash(Scanner& s, const std::wstring& path, int cat, int safety, Node* owner) {
    Trash* t = new Trash();
    t->path = path; t->cat = cat; t->safety = safety; t->owner = owner;
    s.trashes.push_back(t);
    return t;
}

void add_log(Scanner& s, const std::wstring& path, uint64_t bytes, uint32_t mt) {
    std::wstring lp = lower(path);
    if (lp.find(L"\\.claude\\projects") != std::wstring::npos) return;
    // dane aplikacji (np. dzienniki transakcji w data\) nigdy nie sa "pewne"
    int safety = (s.now - mt > 30u * 1440u && lp.find(L"\\data\\") == std::wstring::npos) ? SAFE : CHECK;
    std::ostringstream o;
    o << "T\t" << utf8(path) << '\t' << CAT_LOGS << '\t' << safety << '\t' << bytes << "\t1\t" << mt << '\t';
    std::lock_guard<std::mutex> lk(s.mu);
    s.logs.push_back(Record{o.str(), {}});
}

struct Child { std::wstring name; DWORD attrs; };

void process(Scanner& s, const Work& w) {
    std::wstring base = longp(w.path);
    WIN32_FIND_DATAW fd;
    HANDLE h = FindFirstFileExW((base + L"\\*").c_str(), FindExInfoBasic, &fd, FindExSearchNameMatch, nullptr, FIND_FIRST_EX_LARGE_FETCH);
    s.seen_dirs++;
    if (h == INVALID_HANDLE_VALUE) return;
    std::vector<Child> dirs;
    std::vector<TopFile> top;
    std::array<uint64_t, LANGS> lang{};
    uint64_t bytes = 0;
    uint32_t files = 0, newest = 0, markers = 0;
    const auto& table = ext_table();
    size_t trash_prefix = w.trash ? w.trash->path.size() + 1 : 0;
    do {
        if (!wcscmp(fd.cFileName, L".") || !wcscmp(fd.cFileName, L"..")) continue;
        if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) {
            if (!wcscmp(fd.cFileName, L".git")) markers |= M_GIT;
            dirs.push_back(Child{fd.cFileName, fd.dwFileAttributes});
            continue;
        }
        if (fd.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) continue;
        // pliki tylko w chmurze (OneDrive) nie zajmuja miejsca na dysku
        bool cloud = (fd.dwFileAttributes & (FILE_ATTRIBUTE_OFFLINE | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_RECALL_ON_OPEN)) != 0;
        uint64_t size = ((uint64_t)fd.nFileSizeHigh << 32) | fd.nFileSizeLow;
        uint64_t alloc = cloud ? 0 : (size + 4095) & ~4095ull;
        uint32_t mt = minutes(fd.ftLastWriteTime);
        bytes += alloc;
        files++;
        newest = std::max(newest, mt);
        if (w.trash) {
            if (alloc >= TOP_MIN) top.push_back(TopFile{(w.path + L"\\" + fd.cFileName).substr(trash_prefix), alloc, mt});
            continue;
        }
        if (w.temp_root) {
            if (alloc >= TOP_MIN) top.push_back(TopFile{fd.cFileName, alloc, mt});
            continue;
        }
        std::wstring n = lower(fd.cFileName);
        if (alloc >= LOG_MIN && (ends_with(n, L".log") || ends_with(n, L".dmp") || ends_with(n, L".mdmp"))) {
            add_log(s, w.path + L"\\" + fd.cFileName, alloc, mt);
            bytes -= alloc;
            continue;
        }
        if (!w.node) continue;
        markers |= marker_of(n);
        size_t dot = n.rfind(L'.');
        if (dot == std::wstring::npos || dot == 0) continue;
        auto it = table.find(n.substr(dot + 1));
        if (it == table.end()) continue;
        int l = it->second;
        if (l == 19) markers |= M_PINE;
        if (l == 18) markers |= M_MQL;
        if (l == 0) markers |= M_JS;
        if (l == 9) markers |= M_KT;
        // kod zewnetrzny i wygenerowany nie zmienia proporcji jezykow projektu
        if (!w.vendored && !ends_with(n, L".min.js") && !ends_with(n, L".d.ts") && !ends_with(n, L".bundle.js")) lang[l] += size;
    } while (FindNextFileW(h, &fd));
    FindClose(h);
    s.seen_files += files;
    s.seen_bytes += bytes;

    // Pliki luzem w Temp sa jedna pozycja (X\*); foldery w Temp to osobne pozycje.
    Trash* loose = nullptr;
    if (w.temp_root && files) {
        std::lock_guard<std::mutex> lk(s.mu);
        auto place = s.explicit_paths.find(lower(w.path));
        loose = new_trash(s, w.path + L"\\*", place != s.explicit_paths.end() ? place->second.cat : CAT_TEMP, SAFE, nullptr);
    }
    Trash* sink = w.trash ? w.trash : loose;
    if (sink) {
        std::lock_guard<std::mutex> lk(sink->mu);
        sink->bytes += bytes;
        sink->files += files;
        sink->newest = std::max(sink->newest, newest);
        add_top(sink, top);
    } else if (w.node) {
        w.node->bytes = bytes;
        w.node->files = files;
        w.node->newest = newest;
        w.node->markers = markers;
        for (int i = 0; i < LANGS; i++)
            if (lang[i]) w.node->langs.emplace_back((uint8_t)i, lang[i]);
    } else if (w.owner) {
        w.owner->extra += bytes;
    }

    bool drive_root = w.path.size() <= 3;
    std::wstring lower_path = lower(w.path);
    std::vector<Work> next;
    next.reserve(dirs.size());
    std::lock_guard<std::mutex> lk(s.mu);
    for (const auto& d : dirs) {
        if (d.attrs & FILE_ATTRIBUTE_REPARSE_POINT) continue;  // junction / symlink: bez petli i podwojnego liczenia
        std::wstring n = lower(d.name), full = w.path + L"\\" + d.name;
        if (w.trash) {
            next.push_back(Work{full, nullptr, nullptr, w.trash, true, false, false});
            continue;
        }
        if (skip_anywhere(n) || (drive_root && skip_at_drive(n))) continue;
        std::wstring lf = lower(full);
        auto ex = s.explicit_paths.find(lf);
        if (ex != s.explicit_paths.end()) {
            if (ex->second.children) {
                next.push_back(Work{full, nullptr, nullptr, nullptr, true, false, true});
            } else {
                next.push_back(Work{full, nullptr, nullptr, new_trash(s, full, ex->second.cat, ex->second.safety, nullptr), true, false, false});
            }
            continue;
        }
        if (w.temp_root) {
            // wpisy "dzieci" (Temp, Temp\claude) maja kategorie folderu nadrzednego
            auto parent = s.explicit_paths.find(lower_path);
            int cat = parent != s.explicit_paths.end() ? parent->second.cat : CAT_TEMP;
            next.push_back(Work{full, nullptr, nullptr, new_trash(s, full, cat, SAFE, nullptr), true, false, false});
            continue;
        }
        int cat = 0, safety = 0;
        if (!w.zone && classify(markers, n, lower_path, full, cat, safety)) {
            next.push_back(Work{full, nullptr, nullptr, new_trash(s, full, cat, safety, w.node), true, false, false});
            continue;
        }
        bool zone = w.zone || zone_name(n);
        Node* node = nullptr;
        if (!zone) {
            node = new Node();
            node->parent = w.node;
            node->name = d.name;
            node->vendored = w.vendored || vendored_name(n);
            s.nodes.push_back(node);
        }
        next.push_back(Work{full, node, zone ? (w.node ? w.node : w.owner) : nullptr, nullptr, zone, node ? node->vendored : false, false});
    }
    for (auto& x : next) s.stack.push_back(std::move(x));
}

void worker(Scanner& s) {
    std::unique_lock<std::mutex> lk(s.mu);
    for (;;) {
        s.cv.wait(lk, [&] { return s.done || !s.stack.empty(); });
        if (s.done) return;
        Work w = std::move(s.stack.back());
        s.stack.pop_back();
        s.busy++;
        lk.unlock();
        process(s, w);
        lk.lock();
        s.busy--;
        if (s.stack.empty() && s.busy == 0) {
            s.done = true;
            s.cv.notify_all();
            return;
        }
        s.cv.notify_all();
    }
}

std::wstring node_path(const Node* n) {
    std::vector<const Node*> chain;
    for (; n; n = n->parent) chain.push_back(n);
    std::wstring p = chain.back()->name;
    if (p.size() == 3 && p[2] == L'\\') p.pop_back();
    for (auto it = chain.rbegin() + 1; it != chain.rend(); ++it) p += L"\\" + (*it)->name;
    return p;
}

int kind_of(uint32_t m) {
    if (m & M_CARGO) return 2;
    if (m & M_PACKAGE) return (m & M_TSCONFIG) ? 1 : 31;
    if (m & M_CSPROJ) return 5;
    if (m & (M_CMAKE | M_VCXPROJ | M_SLN)) return 3;  // C# czy C++ rozstrzygaja bajty kodu
    if (m & M_PYTHON) return 6;
    if (m & M_GO) return 7;
    if (m & M_JAVA) return (m & M_KT) ? 9 : 8;
    if (m & M_PHP) return 10;
    if (m & M_RUBY) return 11;
    if (m & M_DART) return 16;
    if (m & M_PINE) return 19;
    if (m & M_MQL) return 18;
    if (m & M_GIT) return KIND_GIT;  // repo bez znacznika: jezyk z najwiekszej ilosci kodu
    if (m & M_HTML) return (m & M_JS) ? 0 : 12;
    return -1;
}

bool is_mql_home(const std::wstring& name) {
    std::wstring n = lower(name);
    return ends_with(n, L"mql5") || ends_with(n, L"mql4");
}

// Korzenie projektow: odrzuca kolekcje (vcpkg/ports), kontenery i podprojekty.
void pick_projects(Scanner& s, bool full) {
    for (Node* n : s.nodes) {
        // pelny skan: korzenie to dyski; odswiezanie: korzen to sam projekt
        if ((full && !n->parent) || s.not_projects.count(lower(node_path(n)))) continue;
        n->kind = kind_of(n->markers);
    }
    // caly katalog ...MQL5 to jeden projekt, a nie kazdy folder biblioteki standardowej
    for (Node* n : s.nodes) {
        if (n->kind != 18) continue;
        for (Node* p = n; p; p = p->parent) {
            if (is_mql_home(p->name)) { n->kind = -1; p->kind = 18; break; }
        }
    }
    std::unordered_map<Node*, std::array<uint32_t, 33>> kids;
    for (Node* n : s.nodes) {
        if (n->kind < 0 || !n->parent) continue;
        auto& c = kids[n->parent];
        if (n->kind < 32) c[n->kind]++;
        c[32]++;
    }
    std::unordered_set<Node*> collections;
    for (auto& kv : kids) {
        auto& c = kv.second;
        if (c[32] < 20 || s.not_projects.count(lower(node_path(kv.first)))) continue;
        int best = (int)(std::max_element(c.begin(), c.begin() + 32) - c.begin());
        if (c[best] * 10 >= c[32] * 9) { collections.insert(kv.first); kv.first->kind = best; }
    }
    auto inside = [](Node* n, const std::unordered_set<Node*>& set) {
        for (Node* p = n->parent; p; p = p->parent) if (set.count(p)) return true;
        return false;
    };
    if (!collections.empty())
        for (Node* n : s.nodes) if (n->kind >= 0 && inside(n, collections)) n->kind = -1;
    std::unordered_map<Node*, uint32_t> direct, nested_count;
    std::unordered_map<Node*, std::unordered_set<int>> nested_kinds;
    for (Node* n : s.nodes) {
        if (n->kind < 0) continue;
        if (n->parent && n->parent->kind >= 0) direct[n->parent]++;
        // pliki index.html w podfolderach nie czynia z projektu zbiornika
        if (n->kind == 0 || n->kind == 12) continue;
        for (Node* p = n->parent; p; p = p->parent) {
            if (p->kind < 0) continue;
            nested_count[p]++;
            nested_kinds[p].insert(n->kind);
        }
    }
    // zbiornik (old, new, archiwum): kilka projektow w sobie, zwykle w roznych jezykach;
    // repozytorium git zostaje jednym projektem nawet z wieloma podpakietami
    auto container = [&](Node* n) {
        if (n->markers & M_GIT) return false;
        auto d = direct.find(n);
        if (d != direct.end() && d->second >= 4) return true;
        auto c = nested_count.find(n);
        return c != nested_count.end() && c->second >= 4 && nested_kinds[n].size() >= 2;
    };
    auto strong = [](int k) { return k != 0 && k != 12 && k != 18 && k != 19; };
    std::vector<Node*> drop;
    for (Node* n : s.nodes) {
        if (n->kind < 0) continue;
        if (container(n)) { drop.push_back(n); continue; }
        int depth = 0;
        for (Node* p = n->parent; p; p = p->parent, depth++) {
            if (p->kind < 0 || container(p)) continue;
            // repozytorium obejmuje wszystko pod soba; inny znacznik do 3 poziomow
            if ((p->markers & M_GIT) || (depth < 3 && strong(p->kind))) { drop.push_back(n); break; }
        }
    }
    for (Node* n : drop) n->kind = -1;
}

void emit(Scanner& s, std::vector<Record>& out) {
    struct Agg { uint64_t bytes = 0; uint32_t files = 0, newest = 0; std::array<uint64_t, LANGS> lang{}; };
    std::unordered_map<Node*, Agg> agg;
    // rodzic zawsze przed dzieckiem, wiec przynaleznosc do projektu idzie z gory na dol
    for (Node* n : s.nodes) {
        n->proj = n->kind >= 0 ? n : (n->parent ? n->parent->proj : nullptr);
        if (!n->proj) continue;
        Agg& a = agg[n->proj];
        a.bytes += n->bytes + n->extra.load();
        a.files += n->files;
        a.newest = std::max(a.newest, n->newest);
        for (const auto& l : n->langs) a.lang[l.first] += l.second;
    }
    for (auto& kv : agg) {
        Node* n = kv.first;
        Agg& a = kv.second;
        int kind = n->kind;
        uint64_t cs = a.lang[5], cpp = a.lang[3], c = a.lang[4];
        if (kind == 3) {
            // .sln bywa i w C#, i w C++: decyduje realny kod, a sam .sln bez kodu to C#
            if (cs > cpp + c || (cs + cpp + c == 0 && !(n->markers & (M_VCXPROJ | M_CMAKE)))) kind = 5;
            else if (c > cpp) kind = 4;
        } else if (kind == 31 && a.lang[1] > a.lang[0]) {
            kind = 1;
        } else if (kind == KIND_GIT) {
            kind = -1;
            uint64_t best = 0;
            for (int i = 0; i < 31; i++) if (is_code_lang(i) && a.lang[i] > best) { best = a.lang[i]; kind = i; }
            if (kind < 0) for (int i = 0; i < 31; i++) if (a.lang[i] > best) { best = a.lang[i]; kind = i; }
            if (kind < 0) continue;  // repo bez kodu (notatki, dane)
        }
        std::ostringstream o;
        o << "P\t" << utf8(node_path(n)) << '\t' << kind << '\t' << a.bytes << '\t' << a.files << '\t' << a.newest << '\t' << ((n->markers & M_GIT) ? 1 : 0) << '\t';
        bool first = true;
        for (int i = 0; i < LANGS; i++) {
            if (!a.lang[i]) continue;
            if (!first) o << ',';
            first = false;
            o << i << ':' << a.lang[i];
        }
        out.push_back(Record{o.str(), {}});
    }
    for (Trash* t : s.trashes) {
        uint64_t min_bytes = t->cat == CAT_TEMP || t->cat == CAT_AI || t->cat == CAT_PKG ? 1 : 64 * 1024;
        if (t->bytes < min_bytes) continue;
        Node* proj = t->owner ? t->owner->proj : nullptr;
        std::ostringstream o;
        o << "T\t" << utf8(t->path) << '\t' << t->cat << '\t' << t->safety << '\t' << t->bytes << '\t' << t->files << '\t' << t->newest << '\t' << (proj ? utf8(node_path(proj)) : "");
        Record r{o.str(), {}};
        std::sort(t->top.begin(), t->top.end(), [](const TopFile& a, const TopFile& b) { return a.bytes > b.bytes; });
        if (t->top.size() > TOP_FILES) t->top.resize(TOP_FILES);
        for (const auto& f : t->top) {
            std::ostringstream l;
            l << "F\t" << utf8(f.rel) << '\t' << f.bytes << '\t' << f.mt;
            r.files.push_back(l.str());
        }
        out.push_back(std::move(r));
    }
    for (auto& r : s.logs) out.push_back(std::move(r));
}

}  // namespace

void scan(const std::vector<std::wstring>& roots, bool full, const std::wstring& progress_file, std::vector<Record>& out, uint32_t& dirs_seen) {
    Scanner s;
    known_places(s);
    auto add_root = [&](const std::wstring& path, bool zone) {
        Node* n = nullptr;
        if (!zone) {
            n = new Node();
            n->name = path;
            s.nodes.push_back(n);
        }
        s.stack.push_back(Work{path, n, nullptr, nullptr, zone, false, false});
    };
    if (full) {
        wchar_t drives[256];
        DWORD len = GetLogicalDriveStringsW(255, drives);
        for (wchar_t* d = drives; d < drives + len && *d; d += wcslen(d) + 1)
            if (GetDriveTypeW(d) == DRIVE_FIXED) add_root(std::wstring(d, 2), false);
        // Windows jest pomijany, ale jego Temp sie liczy
        s.stack.push_back(Work{L"C:\\Windows\\Temp", nullptr, nullptr, nullptr, true, false, true});
    } else {
        for (const auto& p : roots) {
            std::wstring lp = lower(p);
            bool zone = lp.find(L"\\appdata\\") != std::wstring::npos || lp.find(L"\\.") != std::wstring::npos;
            if (zone) continue;  // obserwator zglasza tylko foldery projektow
            Node* n = new Node();
            n->name = p;
            s.nodes.push_back(n);
            s.stack.push_back(Work{p, n, nullptr, nullptr, false, false, false});
        }
    }
    if (s.stack.empty()) return;
    unsigned threads = std::max(2u, std::min(6u, std::thread::hardware_concurrency()));
    std::vector<std::thread> pool;
    for (unsigned i = 0; i < threads; i++) pool.emplace_back([&] { worker(s); });
    if (!progress_file.empty()) {
        for (;;) {
            {
                std::unique_lock<std::mutex> lk(s.mu);
                if (s.cv.wait_for(lk, std::chrono::seconds(1), [&] { return s.done; })) break;
            }
            std::ostringstream o;
            o << "skan\t" << s.seen_dirs.load() << '\t' << s.seen_files.load() << '\t' << s.seen_bytes.load();
            write_text(progress_file, o.str());
        }
    }
    for (auto& t : pool) t.join();
    dirs_seen = s.seen_dirs.load();
    pick_projects(s, full);
    emit(s, out);
}
