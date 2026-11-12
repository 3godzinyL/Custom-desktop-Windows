// Indeks - silnik optymalizacji zasobów tła (C). OSOBNY program indeks-optymalizator.exe
// (nie wkompilowany w Rust): robi całą czarną robotę i gada z Rustem na żywo przez potok.
//
// Rust (mózg) wysyła komendy na stdin, C odpowiada po linii na stdout:
//   s <pid>          -> "<ws> <cpu100> <denied>"   (próbka zużycia, bez dotykania procesu)
//   S <pid> <pid>... -> "<ws> <cpu100> <denied>;..." (to samo hurtem - cała grupa procesów)
//   l                -> "<pid> <ppid> <start_ms> <sesja> <ram_prywatny> <cpu100> <exe>\t..."
//                       (WSZYSTKIE procesy jednym zapytaniem do jądra, jak Menedżer zadań -
//                        bez otwierania procesów, więc widać też gry z anty-cheatem i admina)
//   P <pid>...       -> "<ścieżka>\t..."            (pełne ścieżki exe, puste = brak dostępu)
//   a                -> "<pid> <słychać 0|1> ..."   (sesje dźwięku: otwarte + czy realnie gra)
//   L <poz> <pid>... -> "<zwolnione_bajty> <ile_się_udało>" (stopień 0-3 dla całej grupy)
//   e <pid> <0|1>    -> "ok"                        (EcoQoS wł/wył)
//   p <pid> <klasa>  -> "ok"                        (klasa priorytetu)
//   t <pid>          -> "<zwolnione_bajty>"         (oczyść zbiór roboczy = zwolnij RAM)
//   r <pid>          -> "ok"                        (cofnij wszystko do stanu sprzed dotknięcia)
//   q                -> koniec
//
// Stopnie: 1 = EcoQoS + priorytet poniżej normalnego + średni priorytet pamięci,
// 2 i 3 = EcoQoS + bezczynny + bardzo niski priorytet pamięci + oczyszczenie RAM.
// Cofnięcie przywraca ORYGINALNĄ klasę priorytetu (gra na High wraca na High, nie na Normal).
//
// Używa WYŁĄCZNIE oficjalnych API Windows (jak tryb wydajności Menedżera zadań / Process
// Lasso) - żadnego wstrzykiwania kodu do obcych procesów. Działa tylko na procesach tego
// samego użytkownika bez podniesionych uprawnień (do reszty OpenProcess nie wejdzie -> denied).
// Bezpiecznik: gdy Indeks (rodzic) zniknie albo stdin się zamknie, C sam przywraca wszystko.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <psapi.h>
#include <mmdeviceapi.h>
#include <audiopolicy.h>
#include <endpointvolume.h>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <set>
#include <string>
#pragma comment(lib, "psapi.lib")
#pragma comment(lib, "ole32.lib")

#ifndef PROCESS_POWER_THROTTLING_EXECUTION_SPEED
#define PROCESS_POWER_THROTTLING_EXECUTION_SPEED 0x1
#endif

static std::set<unsigned> g_touched;              // procesy, które ograniczyliśmy (do przywrócenia)
static std::map<unsigned, DWORD> g_orig_prio;     // klasa priorytetu sprzed pierwszego dotknięcia
static CRITICAL_SECTION g_lock;                   // pętla komend vs wątek bezpiecznika

static int eco(unsigned pid, int on) {
    HANDLE h = OpenProcess(PROCESS_SET_INFORMATION, FALSE, pid);
    if (!h) return 0;
    PROCESS_POWER_THROTTLING_STATE s;
    ZeroMemory(&s, sizeof(s));
    s.Version = PROCESS_POWER_THROTTLING_CURRENT_VERSION;
    s.ControlMask = PROCESS_POWER_THROTTLING_EXECUTION_SPEED;
    s.StateMask = on ? PROCESS_POWER_THROTTLING_EXECUTION_SPEED : 0;
    int ok = SetProcessInformation(h, ProcessPowerThrottling, &s, sizeof(s)) ? 1 : 0;
    CloseHandle(h);
    return ok;
}

static int prio(unsigned pid, unsigned long cls) {
    HANDLE h = OpenProcess(PROCESS_SET_INFORMATION | PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
    if (!h) return 0;
    if (!g_orig_prio.count(pid)) {
        DWORD c = GetPriorityClass(h);
        g_orig_prio[pid] = c ? c : NORMAL_PRIORITY_CLASS;
    }
    int ok = SetPriorityClass(h, cls) ? 1 : 0;
    CloseHandle(h);
    return ok;
}

// Priorytet stron pamięci: niski = system oddaje je innym w pierwszej kolejności.
static int memprio(unsigned pid, unsigned long lvl) {
    HANDLE h = OpenProcess(PROCESS_SET_INFORMATION, FALSE, pid);
    if (!h) return 0;
    MEMORY_PRIORITY_INFORMATION m;
    m.MemoryPriority = lvl;
    int ok = SetProcessInformation(h, ProcessMemoryPriority, &m, sizeof(m)) ? 1 : 0;
    CloseHandle(h);
    return ok;
}

static unsigned long long trim(unsigned pid) {
    HANDLE h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA, FALSE, pid);
    if (!h) return 0;
    PROCESS_MEMORY_COUNTERS a, b;
    a.cb = b.cb = sizeof(PROCESS_MEMORY_COUNTERS);
    unsigned long long freed = 0;
    if (GetProcessMemoryInfo(h, &a, sizeof(a))) {
        SetProcessWorkingSetSizeEx(h, (SIZE_T)-1, (SIZE_T)-1, 0);  // = EmptyWorkingSet
        if (GetProcessMemoryInfo(h, &b, sizeof(b)) && a.WorkingSetSize > b.WorkingSetSize)
            freed = a.WorkingSetSize - b.WorkingSetSize;
    }
    CloseHandle(h);
    return freed;
}

static void restore(unsigned pid) {
    eco(pid, 0);
    auto it = g_orig_prio.find(pid);
    prio(pid, it != g_orig_prio.end() ? it->second : NORMAL_PRIORITY_CLASS);
    memprio(pid, MEMORY_PRIORITY_NORMAL);
    g_orig_prio.erase(pid);
}

// Stopień dla jednego procesu; zwraca zwolnione bajty, *ok = czy system pozwolił.
static unsigned long long set_level(unsigned pid, int lvl, int* ok) {
    *ok = 1;
    if (lvl <= 0) {
        if (g_touched.count(pid)) restore(pid);
        g_touched.erase(pid);
        return 0;
    }
    if (!eco(pid, 1)) { *ok = 0; return 0; }
    g_touched.insert(pid);
    if (lvl == 1) {
        prio(pid, BELOW_NORMAL_PRIORITY_CLASS);
        memprio(pid, MEMORY_PRIORITY_MEDIUM);
        return 0;
    }
    prio(pid, IDLE_PRIORITY_CLASS);
    memprio(pid, MEMORY_PRIORITY_VERY_LOW);
    return trim(pid);
}

static void sample(unsigned pid, unsigned long long* ws, unsigned long long* cpu, int* denied) {
    *ws = 0; *cpu = 0; *denied = 0;
    HANDLE h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid);
    if (!h) { *denied = 1; return; }
    PROCESS_MEMORY_COUNTERS m; m.cb = sizeof(m);
    if (GetProcessMemoryInfo(h, &m, sizeof(m))) *ws = m.WorkingSetSize;
    FILETIME c, e, k, u;
    if (GetProcessTimes(h, &c, &e, &k, &u)) {
        unsigned long long kk = ((unsigned long long)k.dwHighDateTime << 32) | k.dwLowDateTime;
        unsigned long long uu = ((unsigned long long)u.dwHighDateTime << 32) | u.dwLowDateTime;
        *cpu = kk + uu;
    }
    CloseHandle(h);
}

// Struktura z NtQuerySystemInformation(SystemProcessInformation) - początek jest stały od
// Windows Vista; używamy tylko pól do WorkingSetSize.
struct SpInfo {
    ULONG NextEntryOffset;
    ULONG NumberOfThreads;
    LARGE_INTEGER WorkingSetPrivateSize;
    ULONG HardFaultCount;
    ULONG NumberOfThreadsHighWatermark;
    ULONGLONG CycleTime;
    LARGE_INTEGER CreateTime;
    LARGE_INTEGER UserTime;
    LARGE_INTEGER KernelTime;
    USHORT NameLength;
    USHORT NameMaximumLength;
    PWSTR NameBuffer;
    LONG BasePriority;
    HANDLE UniqueProcessId;
    HANDLE InheritedFromUniqueProcessId;
    ULONG HandleCount;
    ULONG SessionId;
    ULONG_PTR UniqueProcessKey;
    SIZE_T PeakVirtualSize;
    SIZE_T VirtualSize;
    ULONG PageFaultCount;
    SIZE_T PeakWorkingSetSize;
    SIZE_T WorkingSetSize;
};
typedef LONG(NTAPI* NtQsi)(ULONG, PVOID, ULONG, PULONG);

static void list_processes() {
    static NtQsi qsi = (NtQsi)GetProcAddress(GetModuleHandleW(L"ntdll.dll"), "NtQuerySystemInformation");
    static std::string buf(1 << 20, '\0');
    if (!qsi) { fputs("\n", stdout); return; }
    ULONG need = 0;
    LONG st;
    while ((st = qsi(5, &buf[0], (ULONG)buf.size(), &need)) == (LONG)0xC0000004L)
        buf.resize((need ? need : (ULONG)buf.size()) * 2);
    if (st < 0) { fputs("\n", stdout); return; }
    std::string out;
    char name[MAX_PATH * 3];
    char head[160];
    for (const char* p = buf.data();;) {
        const SpInfo* e = (const SpInfo*)p;
        unsigned pid = (unsigned)(ULONG_PTR)e->UniqueProcessId;
        if (pid) {
            int n = e->NameBuffer ? WideCharToMultiByte(CP_UTF8, 0, e->NameBuffer, e->NameLength / 2, name, sizeof(name) - 1, nullptr, nullptr) : 0;
            name[n > 0 ? n : 0] = 0;
            for (char* q = name; *q; q++) if (*q == '\t' || *q == '\n' || *q == '\r') *q = ' ';
            snprintf(head, sizeof(head), "%u %u %llu %lu %llu %llu ", pid, (unsigned)(ULONG_PTR)e->InheritedFromUniqueProcessId,
                     (unsigned long long)e->CreateTime.QuadPart / 10000ULL, e->SessionId,
                     (unsigned long long)e->WorkingSetPrivateSize.QuadPart,
                     (unsigned long long)(e->UserTime.QuadPart + e->KernelTime.QuadPart));
            out += head; out += name; out += '\t';
        }
        if (!e->NextEntryOffset) break;
        p += e->NextEntryOffset;
    }
    out += '\n';
    fputs(out.c_str(), stdout);
}

static void paths(char* q) {
    std::string out;
    char* end;
    wchar_t w[MAX_PATH * 2];
    char u[MAX_PATH * 6];
    for (unsigned long v = strtoul(q, &end, 10); end != q; q = end, v = strtoul(q, &end, 10)) {
        u[0] = 0;
        HANDLE h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, (DWORD)v);
        if (h) {
            DWORD n = MAX_PATH * 2;
            if (QueryFullProcessImageNameW(h, 0, w, &n)) {
                int m = WideCharToMultiByte(CP_UTF8, 0, w, (int)n, u, sizeof(u) - 1, nullptr, nullptr);
                u[m > 0 ? m : 0] = 0;
            }
            CloseHandle(h);
        }
        out += u; out += '\t';
    }
    out += '\n';
    fputs(out.c_str(), stdout);
}

// Procesy z aktywną sesją dźwięku na domyślnym wyjściu (muzyka, wideo, rozmowa).
static void audio_pids() {
    static bool com = false;
    if (!com) { CoInitializeEx(nullptr, COINIT_MULTITHREADED); com = true; }
    std::string out;
    IMMDeviceEnumerator* en = nullptr;
    if (SUCCEEDED(CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL, __uuidof(IMMDeviceEnumerator), (void**)&en))) {
        IMMDevice* dev = nullptr;
        if (SUCCEEDED(en->GetDefaultAudioEndpoint(eRender, eMultimedia, &dev))) {
            IAudioSessionManager2* mgr = nullptr;
            if (SUCCEEDED(dev->Activate(__uuidof(IAudioSessionManager2), CLSCTX_ALL, nullptr, (void**)&mgr))) {
                IAudioSessionEnumerator* se = nullptr;
                if (SUCCEEDED(mgr->GetSessionEnumerator(&se))) {
                    int n = 0;
                    se->GetCount(&n);
                    for (int i = 0; i < n; i++) {
                        IAudioSessionControl* c = nullptr;
                        if (FAILED(se->GetSession(i, &c))) continue;
                        AudioSessionState st;
                        IAudioSessionControl2* c2 = nullptr;
                        if (SUCCEEDED(c->GetState(&st)) && st == AudioSessionStateActive &&
                            SUCCEEDED(c->QueryInterface(__uuidof(IAudioSessionControl2), (void**)&c2))) {
                            DWORD pid = 0;
                            if (SUCCEEDED(c2->GetProcessId(&pid)) && pid) {
                                // otwarty strumień to jeszcze nie dźwięk (Discord trzyma go stale) -
                                // miernik szczytu mówi, czy coś realnie słychać
                                float peak = 0.f;
                                IAudioMeterInformation* m = nullptr;
                                if (SUCCEEDED(c->QueryInterface(__uuidof(IAudioMeterInformation), (void**)&m))) {
                                    m->GetPeakValue(&peak);
                                    m->Release();
                                }
                                char b[32];
                                snprintf(b, sizeof(b), "%lu %d ", pid, peak > 0.0005f ? 1 : 0);
                                out += b;
                            }
                            c2->Release();
                        }
                        c->Release();
                    }
                    se->Release();
                }
                mgr->Release();
            }
            dev->Release();
        }
        en->Release();
    }
    out += '\n';
    fputs(out.c_str(), stdout);
}

static void restore_all() {
    for (unsigned pid : g_touched) restore(pid);
    g_touched.clear();
}

// Gdy rodzic (Indeks) zniknie - przywróć wszystko i zakończ (bezpiecznik po awarii).
static DWORD WINAPI watch_parent(LPVOID p) {
    WaitForSingleObject((HANDLE)p, INFINITE);
    EnterCriticalSection(&g_lock);
    restore_all();
    fflush(stdout);
    ExitProcess(0);
    return 0;
}

static char g_line[1 << 16];

int wmain(int argc, wchar_t** argv) {
    InitializeCriticalSection(&g_lock);
    int parent = 0;
    for (int i = 1; i + 1 < argc; i++)
        if (!wcscmp(argv[i], L"--rodzic")) parent = _wtoi(argv[i + 1]);
    if (parent) {
        HANDLE ph = OpenProcess(SYNCHRONIZE, FALSE, (DWORD)parent);
        if (ph) {
            DWORD tid;
            CreateThread(nullptr, 0, watch_parent, ph, 0, &tid);
        }
    }
    while (fgets(g_line, sizeof(g_line), stdin)) {
        char* p = g_line;
        if ((unsigned char)p[0] == 0xEF && (unsigned char)p[1] == 0xBB && (unsigned char)p[2] == 0xBF) p += 3;
        unsigned pid = 0, arg = 0;
        char cmd = p[0];
        if (cmd == 'q') break;
        EnterCriticalSection(&g_lock);
        if (cmd == 's') {
            sscanf(p + 1, "%u", &pid);
            unsigned long long ws, cpu; int den;
            sample(pid, &ws, &cpu, &den);
            printf("%llu %llu %d\n", ws, cpu, den);
        } else if (cmd == 'S') {
            std::string out;
            char b[80];
            char* q = p + 1;
            char* end;
            for (unsigned long v = strtoul(q, &end, 10); end != q; q = end, v = strtoul(q, &end, 10)) {
                unsigned long long ws, cpu; int den;
                sample((unsigned)v, &ws, &cpu, &den);
                snprintf(b, sizeof(b), "%llu %llu %d;", ws, cpu, den);
                out += b;
            }
            out += '\n';
            fputs(out.c_str(), stdout);
        } else if (cmd == 'l') {
            list_processes();
        } else if (cmd == 'a') {
            audio_pids();
        } else if (cmd == 'L') {
            char* q = p + 1;
            char* end;
            long lvl = strtol(q, &end, 10);
            unsigned long long freed = 0;
            int oks = 0;
            if (end != q) {
                q = end;
                for (unsigned long v = strtoul(q, &end, 10); end != q; q = end, v = strtoul(q, &end, 10)) {
                    int ok = 0;
                    freed += set_level((unsigned)v, (int)lvl, &ok);
                    oks += ok;
                }
            }
            printf("%llu %d\n", freed, oks);
        } else if (cmd == 'P') {
            paths(p + 1);
        } else if (cmd == 'e') {
            sscanf(p + 1, "%u %u", &pid, &arg);
            eco(pid, (int)arg);
            if (arg) g_touched.insert(pid);
            printf("ok\n");
        } else if (cmd == 'p') {
            sscanf(p + 1, "%u %u", &pid, &arg);
            prio(pid, arg);
            g_touched.insert(pid);
            printf("ok\n");
        } else if (cmd == 't') {
            sscanf(p + 1, "%u", &pid);
            printf("%llu\n", trim(pid));
        } else if (cmd == 'r') {
            sscanf(p + 1, "%u", &pid);
            restore(pid);
            g_touched.erase(pid);
            printf("ok\n");
        } else {
            printf("?\n");
        }
        fflush(stdout);
        LeaveCriticalSection(&g_lock);
    }
    EnterCriticalSection(&g_lock);
    restore_all();  // stdin zamknięty (rodzic zniknął) albo 'q' - cofnij wszystko
    return 0;
}
