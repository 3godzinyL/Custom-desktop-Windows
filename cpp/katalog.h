// Silnik katalogu (C++): wspolne typy i narzedzia. Osobny proces indeks-katalog.exe:
// uruchamia sie, przeszukuje dysk, zapisuje katalog.tsv i konczy - pulpit nie trzyma tych danych.
//
// Format katalog.tsv (UTF-8, pola oddzielone tabulatorem):
//   V  wersja  skan_min  czas_ms  liczba_folderow  ostatnia_zmiana_min
//   P  sciezka  jezyk  bajty_bez_smieci  pliki  najnowszy_min  git  jezyk:bajty,jezyk:bajty
//   T  sciezka  kategoria  pewnosc  bajty  pliki  najnowszy_min  sciezka_projektu
//   F  sciezka_wzgledna  bajty  data_min         (nalezy do poprzedniego T)
#pragma once
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <stdint.h>
#include <string>
#include <vector>

enum Category {
    CAT_NODE = 0, CAT_RUST, CAT_NATIVE, CAT_DOTNET, CAT_PYTHON, CAT_WEB, CAT_JVM,
    CAT_TEMP, CAT_AI, CAT_PKG, CAT_LOGS,
};

constexpr int SAFE = 1;    // da sie odtworzyc jedna komenda
constexpr int CHECK = 2;   // lepiej obejrzec przed usunieciem

inline std::wstring lower(const std::wstring& s) {
    std::wstring o(s);
    for (auto& c : o) if (c >= L'A' && c <= L'Z') c = (wchar_t)(c + 32);
    return o;
}

inline std::string utf8(const std::wstring& s) {
    if (s.empty()) return {};
    int n = WideCharToMultiByte(CP_UTF8, 0, s.data(), (int)s.size(), nullptr, 0, nullptr, nullptr);
    std::string o((size_t)n, '\0');
    WideCharToMultiByte(CP_UTF8, 0, s.data(), (int)s.size(), &o[0], n, nullptr, nullptr);
    return o;
}

inline std::wstring wide(const std::string& s) {
    if (s.empty()) return {};
    int n = MultiByteToWideChar(CP_UTF8, 0, s.data(), (int)s.size(), nullptr, 0);
    std::wstring o((size_t)n, L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s.data(), (int)s.size(), &o[0], n);
    return o;
}

inline std::wstring env(const wchar_t* name) {
    wchar_t buf[1024];
    DWORD n = GetEnvironmentVariableW(name, buf, 1024);
    return n && n < 1024 ? std::wstring(buf, n) : std::wstring();
}

// Sciezka z prefiksem \\?\ (foldery node_modules czesto przekraczaja MAX_PATH).
inline std::wstring longp(const std::wstring& p) {
    if (p.rfind(L"\\\\", 0) == 0) return p;
    return L"\\\\?\\" + p;
}

inline uint32_t minutes(const FILETIME& f) {
    return (uint32_t)((((uint64_t)f.dwHighDateTime << 32) | f.dwLowDateTime) / 600000000ull);
}

inline uint32_t now_minutes() {
    FILETIME f;
    GetSystemTimeAsFileTime(&f);
    return minutes(f);
}

// Czy `p` to `root` albo lezy pod nim (obie sciezki male litery, bez koncowego \).
inline bool under(const std::wstring& p, const std::wstring& root) {
    if (p.size() < root.size() || p.compare(0, root.size(), root) != 0) return false;
    return p.size() == root.size() || p[root.size()] == L'\\';
}

inline std::vector<std::string> split_tabs(const std::string& s) {
    std::vector<std::string> v;
    size_t a = 0;
    for (;;) {
        size_t b = s.find('\t', a);
        if (b == std::string::npos) { v.push_back(s.substr(a)); return v; }
        v.push_back(s.substr(a, b - a));
        a = b + 1;
    }
}

// Rekord katalogu: wiersz P/T z wierszami F, trzymany w postaci tekstowej przy scalaniu.
struct Record {
    std::string line;
    std::vector<std::string> files;
    char type() const { return line.empty() ? 0 : line[0]; }
    std::wstring path() const {
        auto f = split_tabs(line);
        return f.size() > 1 ? wide(f[1]) : std::wstring();
    }
};

struct Catalog {
    std::string header;
    std::vector<Record> records;
};

bool read_catalog(const std::wstring& file, Catalog& out);
bool write_catalog(const std::wstring& file, const Catalog& c);
void write_text(const std::wstring& file, const std::string& text);

// katalog_skan.cpp: przeszukuje `roots` i dopisuje rekordy P/T. `full` = caly komputer.
void scan(const std::vector<std::wstring>& roots, bool full, const std::wstring& progress_file, std::vector<Record>& out, uint32_t& dirs_seen);
