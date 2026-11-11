// Indeks - silnik optymalizacji zasobów tła (C). OSOBNY program indeks-optymalizator.exe
// (nie wkompilowany w Rust): robi całą czarną robotę i gada z Rustem na żywo przez potok.
//
// Rust (mózg) wysyła komendy na stdin, C odpowiada po linii na stdout:
//   s <pid>          -> "<ws> <cpu100> <denied>"   (próbka zużycia, bez dotykania procesu)
//   e <pid> <0|1>    -> "ok"                        (EcoQoS wł/wył)
//   p <pid> <klasa>  -> "ok"                        (klasa priorytetu)
//   t <pid>          -> "<zwolnione_bajty>"         (oczyść zbiór roboczy = zwolnij RAM)
//   r <pid>          -> "ok"                        (cofnij: EcoQoS wył + normalny priorytet)
//   q                -> koniec
//
// Używa WYŁĄCZNIE oficjalnych API Windows (jak tryb wydajności Menedżera zadań / Process
// Lasso) - żadnego wstrzykiwania kodu do obcych procesów. Działa tylko na procesach tego
// samego użytkownika bez podniesionych uprawnień (do reszty OpenProcess nie wejdzie -> denied).
// Bezpiecznik: gdy Indeks (rodzic) zniknie albo stdin się zamknie, C sam przywraca wszystko.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <psapi.h>
#include <cstdio>
#include <cstring>
#include <set>
#pragma comment(lib, "psapi.lib")

#ifndef PROCESS_POWER_THROTTLING_EXECUTION_SPEED
#define PROCESS_POWER_THROTTLING_EXECUTION_SPEED 0x1
#endif

static std::set<unsigned> g_touched;  // procesy, które ograniczyliśmy (do przywrócenia)

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
    HANDLE h = OpenProcess(PROCESS_SET_INFORMATION, FALSE, pid);
    if (!h) return 0;
    int ok = SetPriorityClass(h, cls) ? 1 : 0;
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
    prio(pid, NORMAL_PRIORITY_CLASS);
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

static void restore_all() {
    for (unsigned pid : g_touched) restore(pid);
    g_touched.clear();
}

// Gdy rodzic (Indeks) zniknie - przywróć wszystko i zakończ (bezpiecznik po awarii).
static DWORD WINAPI watch_parent(LPVOID p) {
    WaitForSingleObject((HANDLE)p, INFINITE);
    restore_all();
    fflush(stdout);
    ExitProcess(0);
    return 0;
}

int wmain(int argc, wchar_t** argv) {
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
    char line[256];
    while (fgets(line, sizeof(line), stdin)) {
        char* p = line;
        // pomiń ewentualny BOM UTF-8 (gdy ktoś karmi tekstem z PowerShella)
        if ((unsigned char)p[0] == 0xEF && (unsigned char)p[1] == 0xBB && (unsigned char)p[2] == 0xBF) p += 3;
        unsigned pid = 0, arg = 0;
        char cmd = p[0];
        if (cmd == 'q') break;
        if (cmd == 's') {
            sscanf(p + 1, "%u", &pid);
            unsigned long long ws, cpu; int den;
            sample(pid, &ws, &cpu, &den);
            printf("%llu %llu %d\n", ws, cpu, den);
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
    }
    restore_all();  // stdin zamknięty (rodzic zniknął) albo 'q' - cofnij wszystko
    return 0;
}
