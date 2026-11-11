// Rdzen indeksu plikow: plaska tablica wpisow + wspolny bufor nazw (UTF-8).
// Wyszukiwanie: rownolegle przejscie po wszystkich zywych wpisach, dopasowanie
// podciagow bez rozrozniania wielkosci liter (ASCII + Latin-1 + Latin Extended-A,
// czyli m.in. polskie litery), ranking: dokladna nazwa > prefiks > poczatek slowa > srodek.
#include "search.h"

#include <algorithm>
#include <cstring>
#include <mutex>
#include <shared_mutex>
#include <string>
#include <thread>
#include <unordered_map>
#include <vector>

namespace {

constexpr uint32_t F_DIR = IX_DIR;
constexpr uint32_t F_DELETED = 1u << 8;
constexpr uint32_t F_ASCII = 1u << 9;

struct Entry {
    uint32_t name_off;
    uint32_t name_len;
    uint32_t flags;
    uint32_t parent;
    uint32_t first_child;
    uint32_t next_sib;
    uint32_t mtime;
};

struct Hit {
    uint32_t score;
    uint32_t len;
    uint32_t id;
};

unsigned char g_lower[256];
struct LowerInit {
    LowerInit() {
        for (int i = 0; i < 256; i++) g_lower[i] = (unsigned char)((i >= 'A' && i <= 'Z') ? i + 32 : i);
    }
} g_lower_init;

uint32_t fold_cp(uint32_t c) {
    if (c < 0x80) return g_lower[c];
    if (c >= 0xC0 && c <= 0xDE && c != 0xD7) return c + 32;
    if ((c >= 0x100 && c <= 0x12F) || (c >= 0x132 && c <= 0x137) || (c >= 0x14A && c <= 0x177))
        return (c % 2 == 0) ? c + 1 : c;
    if ((c >= 0x139 && c <= 0x148) || (c >= 0x179 && c <= 0x17E))
        return (c % 2 == 1) ? c + 1 : c;
    if (c == 0x178) return 0xFF;
    if (c >= 0x391 && c <= 0x3A9 && c != 0x3A2) return c + 32;  // grecki
    if (c >= 0x410 && c <= 0x42F) return c + 32;                 // cyrylica
    if (c >= 0x400 && c <= 0x40F) return c + 80;
    return c;
}

void put_utf8(std::string& out, uint32_t c) {
    if (c < 0x80) {
        out.push_back((char)c);
    } else if (c < 0x800) {
        out.push_back((char)(0xC0 | (c >> 6)));
        out.push_back((char)(0x80 | (c & 0x3F)));
    } else if (c < 0x10000) {
        out.push_back((char)(0xE0 | (c >> 12)));
        out.push_back((char)(0x80 | ((c >> 6) & 0x3F)));
        out.push_back((char)(0x80 | (c & 0x3F)));
    } else {
        out.push_back((char)(0xF0 | (c >> 18)));
        out.push_back((char)(0x80 | ((c >> 12) & 0x3F)));
        out.push_back((char)(0x80 | ((c >> 6) & 0x3F)));
        out.push_back((char)(0x80 | (c & 0x3F)));
    }
}

// Sklada UTF-8 do malych liter. Niepoprawne bajty przepisuje bez zmian.
void fold_utf8(const char* s, uint32_t n, std::string& out) {
    out.clear();
    const unsigned char* p = (const unsigned char*)s;
    uint32_t i = 0;
    while (i < n) {
        unsigned char b = p[i];
        if (b < 0x80) {
            out.push_back((char)g_lower[b]);
            i++;
            continue;
        }
        uint32_t c = 0, extra = 0;
        if ((b & 0xE0) == 0xC0) { c = b & 0x1F; extra = 1; }
        else if ((b & 0xF0) == 0xE0) { c = b & 0x0F; extra = 2; }
        else if ((b & 0xF8) == 0xF0) { c = b & 0x07; extra = 3; }
        else { out.push_back((char)b); i++; continue; }
        bool ok = true;
        for (uint32_t k = 1; k <= extra; k++) {
            if (i + k >= n || (p[i + k] & 0xC0) != 0x80) { ok = false; break; }
            c = (c << 6) | (p[i + k] & 0x3F);
        }
        if (!ok) { out.push_back((char)b); i++; continue; }
        put_utf8(out, fold_cp(c));
        i += extra + 1;
    }
}

bool is_ascii(const char* s, uint32_t n) {
    for (uint32_t i = 0; i < n; i++)
        if ((unsigned char)s[i] >= 0x80) return false;
    return true;
}

bool word_char(unsigned char c) {
    return (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c >= 0x80;
}

// Szuka igly (juz zlozonej) w tekscie (juz zlozonym). Zwraca pozycje lub -1.
inline long find_folded(const char* hay, uint32_t hn, const std::string& needle) {
    const uint32_t nn = (uint32_t)needle.size();
    if (nn == 0) return 0;
    if (nn > hn) return -1;
    const char first = needle[0];
    const char* end = hay + (hn - nn) + 1;
    const char* p = hay;
    while (p < end) {
        p = (const char*)memchr(p, first, (size_t)(end - p));
        if (!p) return -1;
        if (memcmp(p, needle.data(), nn) == 0) return (long)(p - hay);
        p++;
    }
    return -1;
}

// ---------- jezyki (silnik projektow) ----------

// Rozszerzenie (male litery ASCII, do 8 znakow) -> klucz 64-bit.
inline uint64_t ext_key(const char* s, uint32_t n) {
    if (n == 0 || n > 8) return 0;
    uint64_t k = 0;
    for (uint32_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char)s[i];
        if (c >= 0x80) return 0;
        k = (k << 8) | g_lower[c];
    }
    return k;
}

// Identyfikatory zgodne z tablica LANGS w src/profiles.rs.
const struct { const char* ext; uint8_t lang; } EXT_LANGS[] = {
    {"js", 0}, {"mjs", 0}, {"cjs", 0}, {"jsx", 0},
    {"ts", 1}, {"tsx", 1}, {"mts", 1}, {"cts", 1},
    {"rs", 2},
    {"cpp", 3}, {"cc", 3}, {"cxx", 3}, {"hpp", 3}, {"hh", 3}, {"hxx", 3}, {"inl", 3},
    {"c", 4}, {"h", 4},
    {"cs", 5},
    {"py", 6}, {"pyw", 6}, {"ipynb", 6},
    {"go", 7}, {"java", 8}, {"kt", 9}, {"kts", 9}, {"php", 10}, {"rb", 11},
    {"html", 12}, {"htm", 12},
    {"css", 13}, {"scss", 13}, {"sass", 13}, {"less", 13},
    {"sh", 14}, {"bash", 14}, {"ps1", 14}, {"psm1", 14}, {"bat", 14}, {"cmd", 14},
    {"lua", 15}, {"dart", 16}, {"swift", 17},
    {"mq4", 18}, {"mq5", 18}, {"mqh", 18},
    {"pine", 19}, {"vue", 20}, {"svelte", 21}, {"sql", 22}, {"asm", 23}, {"s", 23},
    {"zig", 24}, {"r", 25}, {"scala", 26}, {"hs", 27}, {"jl", 28}, {"wat", 29},
    {"glsl", 30}, {"hlsl", 30}, {"wgsl", 30}, {"frag", 30}, {"vert", 30},
};

int lang_of(const char* name, uint32_t len) {
    static const std::unordered_map<uint64_t, uint8_t> table = [] {
        std::unordered_map<uint64_t, uint8_t> m;
        for (const auto& e : EXT_LANGS) m[ext_key(e.ext, (uint32_t)strlen(e.ext))] = e.lang;
        return m;
    }();
    uint32_t dot = len;
    while (dot > 0 && name[dot - 1] != '.') dot--;
    if (dot <= 1) return -1;  // brak rozszerzenia albo ".plik"
    uint64_t k = ext_key(name + dot, len - dot);
    if (!k) return -1;
    auto it = table.find(k);
    return it == table.end() ? -1 : it->second;
}

bool name_is(const char* s, uint32_t n, const char* want) {
    uint32_t wl = (uint32_t)strlen(want);
    if (n != wl) return false;
    for (uint32_t i = 0; i < n; i++)
        if (g_lower[(unsigned char)s[i]] != (unsigned char)want[i]) return false;
    return true;
}

bool name_ends(const char* s, uint32_t n, const char* suffix) {
    uint32_t sl = (uint32_t)strlen(suffix);
    if (n < sl) return false;
    for (uint32_t i = 0; i < sl; i++)
        if (g_lower[(unsigned char)s[n - sl + i]] != (unsigned char)suffix[i]) return false;
    return true;
}

// Bity znacznikow - zgodne z src/profiles.rs.
uint32_t marker_of(const char* s, uint32_t n, bool dir) {
    if (dir) return name_is(s, n, ".git") ? (1u << 12) : 0;
    if (name_is(s, n, "package.json")) return 1u << 0;
    if (name_is(s, n, "cargo.toml")) return 1u << 1;
    if (name_is(s, n, "cmakelists.txt")) return 1u << 2;
    if (name_ends(s, n, ".sln")) return 1u << 3;
    if (name_ends(s, n, ".vcxproj")) return 1u << 4;
    if (name_ends(s, n, ".csproj")) return 1u << 5;
    if (name_is(s, n, "pyproject.toml") || name_is(s, n, "requirements.txt") || name_is(s, n, "setup.py")) return 1u << 6;
    if (name_is(s, n, "go.mod")) return 1u << 7;
    if (name_is(s, n, "pom.xml") || name_is(s, n, "build.gradle") || name_is(s, n, "build.gradle.kts")) return 1u << 8;
    if (name_is(s, n, "composer.json")) return 1u << 9;
    if (name_is(s, n, "gemfile")) return 1u << 10;
    if (name_is(s, n, "pubspec.yaml")) return 1u << 11;
    if (name_is(s, n, "tsconfig.json")) return 1u << 13;
    if (name_is(s, n, "index.html")) return 1u << 14;
    if (name_is(s, n, "makefile")) return 1u << 15;
    return 0;
}

}  // namespace

struct ix_index {
    std::shared_mutex mu;
    std::vector<Entry> entries;
    std::vector<char> names;
    std::vector<uint32_t> roots;
    uint32_t live = 0;

    bool alive(uint32_t id) const { return id < entries.size() && !(entries[id].flags & F_DELETED); }

    uint32_t store_name(const char* s, uint32_t n) {
        uint32_t off = (uint32_t)names.size();
        names.insert(names.end(), s, s + n);
        return off;
    }

    bool name_eq_ci(uint32_t id, const char* s, uint32_t n, std::string& a, std::string& b) const {
        const Entry& e = entries[id];
        if (e.name_len == n && memcmp(&names[e.name_off], s, n) == 0) return true;
        fold_utf8(&names[e.name_off], e.name_len, a);
        fold_utf8(s, n, b);
        return a == b;
    }
};

extern "C" {

ix_index* ix_create(void) {
    try {
        return new ix_index();
    } catch (...) {
        return nullptr;
    }
}

void ix_destroy(ix_index* ix) { delete ix; }

void ix_clear(ix_index* ix) {
    std::unique_lock<std::shared_mutex> lk(ix->mu);
    std::vector<Entry>().swap(ix->entries);
    std::vector<char>().swap(ix->names);
    ix->roots.clear();
    ix->live = 0;
}

uint32_t ix_add(ix_index* ix, uint32_t parent, const char* name, uint32_t len, uint32_t flags, uint32_t mtime) {
    try {
        std::unique_lock<std::shared_mutex> lk(ix->mu);
        if (parent != IX_NONE && !ix->alive(parent)) return IX_NONE;
        Entry e{};
        e.name_off = ix->store_name(name, len);
        e.name_len = len;
        e.mtime = mtime;
        e.flags = (flags & F_DIR) | (is_ascii(name, len) ? F_ASCII : 0);
        e.parent = parent;
        e.first_child = IX_NONE;
        e.next_sib = IX_NONE;
        uint32_t id = (uint32_t)ix->entries.size();
        if (parent != IX_NONE) {
            e.next_sib = ix->entries[parent].first_child;
            ix->entries[parent].first_child = id;
        } else {
            ix->roots.push_back(id);
        }
        ix->entries.push_back(e);
        ix->live++;
        return id;
    } catch (...) {
        return IX_NONE;
    }
}

uint32_t ix_find_child(ix_index* ix, uint32_t parent, const char* name, uint32_t len) {
    std::shared_lock<std::shared_mutex> lk(ix->mu);
    if (!ix->alive(parent)) return IX_NONE;
    std::string a, b;
    for (uint32_t c = ix->entries[parent].first_child; c != IX_NONE; c = ix->entries[c].next_sib) {
        if (ix->name_eq_ci(c, name, len, a, b)) return c;
    }
    return IX_NONE;
}

void ix_remove(ix_index* ix, uint32_t id) {
    std::unique_lock<std::shared_mutex> lk(ix->mu);
    if (!ix->alive(id)) return;
    uint32_t parent = ix->entries[id].parent;
    if (parent != IX_NONE) {
        uint32_t* link = &ix->entries[parent].first_child;
        while (*link != IX_NONE && *link != id) link = &ix->entries[*link].next_sib;
        if (*link == id) *link = ix->entries[id].next_sib;
    }
    std::vector<uint32_t> stack{id};
    while (!stack.empty()) {
        uint32_t cur = stack.back();
        stack.pop_back();
        Entry& e = ix->entries[cur];
        if (e.flags & F_DELETED) continue;
        e.flags |= F_DELETED;
        ix->live--;
        for (uint32_t c = e.first_child; c != IX_NONE; c = ix->entries[c].next_sib) stack.push_back(c);
    }
}

void ix_rename(ix_index* ix, uint32_t id, const char* name, uint32_t len) {
    try {
        std::unique_lock<std::shared_mutex> lk(ix->mu);
        if (!ix->alive(id)) return;
        uint32_t off = ix->store_name(name, len);
        Entry& e = ix->entries[id];
        e.name_off = off;
        e.name_len = len;
        e.flags = (e.flags & ~F_ASCII) | (is_ascii(name, len) ? F_ASCII : 0);
    } catch (...) {
    }
}

uint32_t ix_path(ix_index* ix, uint32_t id, char* buf, uint32_t cap) {
    std::shared_lock<std::shared_mutex> lk(ix->mu);
    if (id >= ix->entries.size()) return 0;
    uint32_t chain[512];
    uint32_t depth = 0;
    for (uint32_t c = id; c != IX_NONE && depth < 512; c = ix->entries[c].parent) chain[depth++] = c;
    std::string path;
    for (uint32_t i = depth; i-- > 0;) {
        const Entry& e = ix->entries[chain[i]];
        if (!path.empty() && path.back() != '\\') path.push_back('\\');
        path.append(&ix->names[e.name_off], e.name_len);
    }
    uint32_t n = (uint32_t)path.size();
    if (buf && cap) memcpy(buf, path.data(), std::min(n, cap));
    return n;
}

uint32_t ix_flags(ix_index* ix, uint32_t id) {
    std::shared_lock<std::shared_mutex> lk(ix->mu);
    if (!ix->alive(id)) return 0;
    return ix->entries[id].flags & F_DIR;
}

uint32_t ix_count(ix_index* ix) {
    std::shared_lock<std::shared_mutex> lk(ix->mu);
    return ix->live;
}

uint64_t ix_memory(ix_index* ix) {
    std::shared_lock<std::shared_mutex> lk(ix->mu);
    return (uint64_t)ix->entries.capacity() * sizeof(Entry) + ix->names.capacity();
}

void ix_touch(ix_index* ix, uint32_t id, uint32_t mtime) {
    std::unique_lock<std::shared_mutex> lk(ix->mu);
    if (ix->alive(id)) ix->entries[id].mtime = mtime;
}

uint32_t ix_profile_dir(ix_index* ix, uint32_t id, ix_profile* out) {
    memset(out, 0, sizeof(*out));
    std::shared_lock<std::shared_mutex> lk(ix->mu);
    if (!ix->alive(id)) return 0;
    // znaczniki tylko w samym folderze
    for (uint32_t c = ix->entries[id].first_child; c != IX_NONE; c = ix->entries[c].next_sib) {
        const Entry& e = ix->entries[c];
        if (e.flags & F_DELETED) continue;
        out->markers |= marker_of(&ix->names[e.name_off], e.name_len, (e.flags & F_DIR) != 0);
    }
    // cale poddrzewo: jezyki, najnowszy plik (foldery pomijane w indeksie, np.
    // node_modules, sa liscmi bez zawartosci - nie zawyzaja wyniku)
    std::vector<uint32_t> stack{id};
    uint32_t visited = 0;
    while (!stack.empty() && visited < 3000000) {
        uint32_t cur = stack.back();
        stack.pop_back();
        for (uint32_t c = ix->entries[cur].first_child; c != IX_NONE; c = ix->entries[c].next_sib) {
            const Entry& e = ix->entries[c];
            visited++;
            if (e.flags & F_DELETED) continue;
            if (e.flags & F_DIR) {
                out->dirs++;
                stack.push_back(c);
                continue;
            }
            out->files++;
            if (e.mtime > out->newest) out->newest = e.mtime;
            int l = lang_of(&ix->names[e.name_off], e.name_len);
            if (l >= 0 && l < IX_LANGS) out->lang[l]++;
        }
    }
    return 1;
}

uint32_t ix_find_projects(ix_index* ix, uint32_t* out_ids, uint8_t* out_kind, uint32_t max) {
    std::shared_lock<std::shared_mutex> lk(ix->mu);
    // 1) znaczniki i rodzaje plikow zebrane w folderach nadrzednych
    const uint32_t B_PINE = 1u << 16, B_MQL = 1u << 17, B_JS = 1u << 18, B_KT = 1u << 19;
    std::unordered_map<uint32_t, uint32_t> dirs;
    const uint32_t n = (uint32_t)ix->entries.size();
    for (uint32_t id = 0; id < n; id++) {
        const Entry& e = ix->entries[id];
        if ((e.flags & (F_DELETED | F_DIR)) || e.parent == IX_NONE) continue;
        const char* name = &ix->names[e.name_off];
        uint32_t bits = marker_of(name, e.name_len, false);
        int l = lang_of(name, e.name_len);
        if (l == 19) bits |= B_PINE;
        if (l == 18) bits |= B_MQL;
        if (l == 0) bits |= B_JS;
        if (l == 9) bits |= B_KT;
        if (bits) dirs[e.parent] |= bits;
    }
    // 2) rodzaj projektu - jak detect() w src/profiles.rs, ale tylko ze znacznikow
    auto kind_of = [&](uint32_t m) -> int {
        if (m & (1u << 1)) return 2;                                   // Cargo.toml -> Rust
        if (m & (1u << 0)) return (m & (1u << 13)) ? 1 : 31;           // package.json -> TS / Node
        if (m & (1u << 5)) return 5;                                   // .csproj -> C#
        if (m & ((1u << 2) | (1u << 4) | (1u << 3))) return 3;         // CMake / vcxproj / sln -> C++
        if (m & (1u << 6)) return 6;                                   // Python
        if (m & (1u << 7)) return 7;                                   // Go
        if (m & (1u << 8)) return (m & B_KT) ? 9 : 8;                  // Gradle/Maven -> Kotlin / Java
        if (m & (1u << 9)) return 10;                                  // PHP
        if (m & (1u << 10)) return 11;                                 // Ruby
        if (m & (1u << 11)) return 16;                                 // Dart
        if (m & B_PINE) return 19;                                     // Pine Script
        if (m & B_MQL) return 18;                                      // MQL
        if (m & (1u << 14)) return (m & B_JS) ? 0 : 12;                // index.html -> JS / HTML
        return -1;
    };
    // AppData i foldery z kropka (.vscode, .cargo, .nuget...) to pakiety i programy, nie projekty
    auto hidden_path = [&](uint32_t id) {
        for (uint32_t p = id; p != IX_NONE; p = ix->entries[p].parent) {
            const Entry& e = ix->entries[p];
            const char* s = &ix->names[e.name_off];
            if (e.name_len > 1 && s[0] == '.') return true;
            if (e.name_len == 7 && _strnicmp(s, "appdata", 7) == 0) return true;
        }
        return false;
    };
    std::unordered_map<uint32_t, uint8_t> roots;
    for (const auto& kv : dirs) {
        int k = kind_of(kv.second);
        // korzen indeksu (np. profil z przypadkowym package.json) nie jest projektem
        bool is_root = std::find(ix->roots.begin(), ix->roots.end(), kv.first) != ix->roots.end();
        if (k >= 0 && !is_root && ix->alive(kv.first) && !hidden_path(kv.first)) roots[kv.first] = (uint8_t)k;
    }
    // MQL: caly katalog ...MQL5 / ...MQL4 (terminal albo jego kopia) to jeden projekt,
    // a nie kazdy folder biblioteki standardowej (Include/Math/Alglib, Arrays...)
    auto mql_home = [&](uint32_t id) -> uint32_t {
        for (uint32_t p = id; p != IX_NONE; p = ix->entries[p].parent) {
            const Entry& e = ix->entries[p];
            const char* s = &ix->names[e.name_off];
            if (e.name_len >= 4 && (_strnicmp(s + e.name_len - 4, "mql5", 4) == 0 || _strnicmp(s + e.name_len - 4, "mql4", 4) == 0)) return p;
        }
        return IX_NONE;
    };
    {
        std::unordered_map<uint32_t, uint8_t> merged;
        for (const auto& kv : roots) {
            uint32_t home = kv.second == 18 ? mql_home(kv.first) : IX_NONE;
            if (home != IX_NONE) merged[home] = 18;
            else merged.emplace(kv.first, kv.second);
        }
        roots.swap(merged);
    }
    // kolekcja: >= 20 projektow jednego rodzaju bezposrednio w folderze (vcpkg/ports,
    // repozytoria przykladow) liczy sie jako jeden projekt
    {
        std::unordered_map<uint32_t, std::vector<uint32_t>> kids;
        for (const auto& kv : roots) {
            uint32_t p = ix->entries[kv.first].parent;
            if (p == IX_NONE) continue;
            auto& c = kids[p];
            if (c.empty()) c.assign(33, 0);
            c[kv.second]++;
            c[32]++;
        }
        std::unordered_map<uint32_t, uint8_t> coll;
        for (const auto& kv : kids) {
            const auto& c = kv.second;
            if (c[32] < 20) continue;
            uint8_t best = (uint8_t)(std::max_element(c.begin(), c.begin() + 32) - c.begin());
            if (c[best] * 10 >= c[32] * 9) coll[kv.first] = best;
        }
        if (!coll.empty()) {
            std::unordered_map<uint32_t, uint8_t> kept;
            for (const auto& kv : roots) {
                bool inside = false;
                for (uint32_t p = ix->entries[kv.first].parent; p != IX_NONE && !inside; p = ix->entries[p].parent)
                    inside = coll.count(p) > 0;
                if (!inside) kept.emplace(kv.first, kv.second);
            }
            for (const auto& kv : coll) {
                bool inside = false;
                for (uint32_t p = ix->entries[kv.first].parent; p != IX_NONE && !inside; p = ix->entries[p].parent)
                    inside = coll.count(p) > 0;
                if (!inside) kept[kv.first] = kv.second;
            }
            roots.swap(kept);
        }
    }
    // folder z >= 4 projektami bezposrednio w sobie (np. Pulpit z package.json) to kontener
    std::unordered_map<uint32_t, uint32_t> direct;
    for (const auto& kv : roots) {
        uint32_t p = ix->entries[kv.first].parent;
        if (p != IX_NONE && roots.count(p)) direct[p]++;
    }
    auto container = [&](uint32_t id) { auto it = direct.find(id); return it != direct.end() && it->second >= 4; };
    // 3) bez zagniezdzonych (client/, packages/x/ do 3 poziomow pod projektem
    //    z mocnym znacznikiem); index.html / Pine / MQL nie ukrywaja podprojektow
    auto strong = [](uint8_t k) { return k != 0 && k != 12 && k != 18 && k != 19; };
    uint32_t found = 0;
    for (const auto& kv : roots) {
        if (container(kv.first)) continue;
        bool nested = false;
        uint32_t p = ix->entries[kv.first].parent;
        for (int depth = 0; depth < 3 && p != IX_NONE; depth++, p = ix->entries[p].parent) {
            auto it = roots.find(p);
            if (it != roots.end() && strong(it->second) && !container(p)) { nested = true; break; }
        }
        if (nested) continue;
        if (found < max) {
            out_ids[found] = kv.first;
            out_kind[found] = kv.second;
        }
        found++;
    }
    return found;
}

void ix_compact(ix_index* ix) {
    try {
        std::unique_lock<std::shared_mutex> lk(ix->mu);
        ix->entries.shrink_to_fit();
        ix->names.shrink_to_fit();
    } catch (...) {
    }
}

uint32_t ix_lookup(ix_index* ix, const char* path, uint32_t len) {
    std::shared_lock<std::shared_mutex> lk(ix->mu);
    std::string want, have, part;
    fold_utf8(path, len, want);
    while (want.size() > 3 && want.back() == '\\') want.pop_back();
    // najdluzszy korzen bedacy prefiksem sciezki (na granicy czlonu)
    uint32_t best = IX_NONE;
    size_t best_len = 0;
    for (uint32_t r : ix->roots) {
        if (!ix->alive(r)) continue;
        const Entry& e = ix->entries[r];
        fold_utf8(&ix->names[e.name_off], e.name_len, have);
        while (have.size() > 3 && have.back() == '\\') have.pop_back();
        if (want.size() < have.size() || want.compare(0, have.size(), have) != 0) continue;
        bool boundary = want.size() == have.size() || want[have.size()] == '\\' || have.back() == '\\';
        if (boundary && have.size() > best_len) {
            best = r;
            best_len = have.size();
        }
    }
    if (best == IX_NONE) return IX_NONE;
    uint32_t id = best;
    size_t i = best_len;
    while (i < want.size()) {
        while (i < want.size() && want[i] == '\\') i++;
        size_t j = i;
        while (j < want.size() && want[j] != '\\') j++;
        if (j == i) break;
        part.assign(want, i, j - i);
        uint32_t found = IX_NONE;
        for (uint32_t c = ix->entries[id].first_child; c != IX_NONE; c = ix->entries[c].next_sib) {
            const Entry& e = ix->entries[c];
            if (e.flags & F_ASCII) {
                if (e.name_len != part.size()) continue;
                bool eq = true;
                for (uint32_t k = 0; k < e.name_len && eq; k++)
                    eq = g_lower[(unsigned char)ix->names[e.name_off + k]] == (unsigned char)part[k];
                if (!eq) continue;
            } else {
                fold_utf8(&ix->names[e.name_off], e.name_len, have);
                if (have != part) continue;
            }
            found = c;
            break;
        }
        if (found == IX_NONE) return IX_NONE;
        id = found;
        i = j;
    }
    return id;
}

uint32_t ix_child_dirs(ix_index* ix, uint32_t id, uint32_t* out, uint32_t max) {
    std::shared_lock<std::shared_mutex> lk(ix->mu);
    if (!ix->alive(id)) return 0;
    uint32_t n = 0;
    for (uint32_t c = ix->entries[id].first_child; c != IX_NONE; c = ix->entries[c].next_sib) {
        const Entry& e = ix->entries[c];
        if ((e.flags & F_DELETED) || !(e.flags & F_DIR)) continue;
        if (n < max) out[n] = c;
        n++;
    }
    return n;
}

uint32_t ix_name(ix_index* ix, uint32_t id, char* buf, uint32_t cap) {
    std::shared_lock<std::shared_mutex> lk(ix->mu);
    if (id >= ix->entries.size()) return 0;
    const Entry& e = ix->entries[id];
    if (buf && cap) memcpy(buf, &ix->names[e.name_off], std::min(e.name_len, cap));
    return e.name_len;
}

uint32_t ix_search(ix_index* ix, const char* query, uint32_t qlen, uint32_t mode,
                   uint32_t* out, uint32_t max, uint32_t* total) {
    if (total) *total = 0;
    std::string folded;
    fold_utf8(query, qlen, folded);
    std::vector<std::string> tokens;
    {
        size_t i = 0;
        while (i < folded.size()) {
            while (i < folded.size() && folded[i] == ' ') i++;
            size_t j = i;
            while (j < folded.size() && folded[j] != ' ') j++;
            if (j > i) tokens.emplace_back(folded.substr(i, j - i));
            i = j;
        }
    }
    if (tokens.empty()) return 0;

    std::shared_lock<std::shared_mutex> lk(ix->mu);
    const uint32_t n = (uint32_t)ix->entries.size();
    if (n == 0) return 0;

    unsigned hw = std::thread::hardware_concurrency();
    unsigned threads = std::max(1u, std::min(hw ? hw : 4u, 8u));
    if (n < 20000) threads = 1;
    std::vector<std::vector<Hit>> parts(threads);
    std::vector<uint32_t> counts(threads, 0);

    auto work = [&](unsigned t) {
        const uint32_t lo = (uint32_t)((uint64_t)n * t / threads);
        const uint32_t hi = (uint32_t)((uint64_t)n * (t + 1) / threads);
        std::string buf;
        std::vector<Hit>& hits = parts[t];
        uint32_t cnt = 0;
        for (uint32_t id = lo; id < hi; id++) {
            const Entry& e = ix->entries[id];
            if (e.flags & F_DELETED) continue;
            if (mode == IX_FIND_DIRS && !(e.flags & F_DIR)) continue;
            if (mode == IX_FIND_FILES && (e.flags & F_DIR)) continue;
            const char* name = &ix->names[e.name_off];
            uint32_t len = e.name_len;
            if (e.parent == IX_NONE) {
                // korzen: dopasuj tylko ostatni czlon sciezki
                uint32_t k = len;
                while (k > 0 && name[k - 1] != '\\') k--;
                name += k;
                len -= k;
            }
            const char* h;
            uint32_t hn;
            if (e.flags & F_ASCII) {
                buf.resize(len);
                for (uint32_t k = 0; k < len; k++) buf[k] = (char)g_lower[(unsigned char)name[k]];
            } else {
                fold_utf8(name, len, buf);
            }
            h = buf.data();
            hn = (uint32_t)buf.size();
            uint32_t score = 0;
            bool all = true;
            for (const std::string& tok : tokens) {
                long pos = find_folded(h, hn, tok);
                if (pos < 0) { all = false; break; }
                if (pos == 0) score += (tok.size() == hn) ? 0 : 1;
                else if (!word_char((unsigned char)h[pos - 1])) score += 2;
                else score += 3;
            }
            if (!all) continue;
            cnt++;
            if (!(e.flags & F_DIR)) score += 1;  // foldery minimalnie wyzej
            hits.push_back(Hit{score, hn, id});
        }
        counts[t] = cnt;
    };

    try {
        if (threads == 1) {
            work(0);
        } else {
            std::vector<std::thread> pool;
            for (unsigned t = 1; t < threads; t++) pool.emplace_back(work, t);
            work(0);
            for (auto& th : pool) th.join();
        }
    } catch (...) {
        return 0;
    }

    std::vector<Hit> all;
    uint32_t tot = 0;
    for (unsigned t = 0; t < threads; t++) {
        tot += counts[t];
        all.insert(all.end(), parts[t].begin(), parts[t].end());
    }
    if (total) *total = tot;
    auto better = [](const Hit& a, const Hit& b) {
        if (a.score != b.score) return a.score < b.score;
        if (a.len != b.len) return a.len < b.len;
        return a.id < b.id;
    };
    uint32_t take = std::min<uint32_t>(max, (uint32_t)all.size());
    std::partial_sort(all.begin(), all.begin() + take, all.end(), better);
    for (uint32_t i = 0; i < take; i++) out[i] = all[i].id;
    return take;
}

}  // extern "C"
