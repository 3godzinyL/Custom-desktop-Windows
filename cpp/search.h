// Rdzen indeksu plikow i wyszukiwania (C++17), wystawiony jako C ABI dla Rusta.
// Wszystkie nazwy w UTF-8. Funkcje sa bezpieczne watkowo (shared_mutex).
#pragma once
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ix_index ix_index;

#define IX_DIR 1u
#define IX_NONE 0xFFFFFFFFu

ix_index* ix_create(void);
void ix_destroy(ix_index* ix);
void ix_clear(ix_index* ix);

// Dodaje wpis. Korzen: parent = IX_NONE, name = pelna sciezka korzenia.
// `mtime` = czas modyfikacji w minutach od 1601 (FILETIME / 600 000 000).
uint32_t ix_add(ix_index* ix, uint32_t parent, const char* name, uint32_t len, uint32_t flags, uint32_t mtime);
// Plik zmieniony (zdarzenie z ReadDirectoryChangesW).
void ix_touch(ix_index* ix, uint32_t id, uint32_t mtime);

// --- profil folderu (silnik projektow) ---
#define IX_LANGS 32
typedef struct ix_profile {
    uint32_t files;
    uint32_t dirs;
    uint32_t newest;          // najnowszy plik w poddrzewie (minuty od 1601)
    uint32_t markers;         // pliki-znaczniki w samym folderze (package.json, Cargo.toml...)
    uint32_t lang[IX_LANGS];  // liczba plikow kodu wg jezyka
} ix_profile;
// Liczy profil poddrzewa wpisu z indeksu w pamieci (bez dysku). 0 = brak wpisu.
uint32_t ix_profile_dir(ix_index* ix, uint32_t id, ix_profile* out);

// Korzenie projektow w calym indeksie (folder z plikiem-znacznikiem albo z plikami
// .pine/.mq5), bez zagniezdzonych. out_kind = jezyk wg tablicy LANGS (src/profiles.rs).
// Zwraca liczbe wszystkich znalezionych (zapisuje max max).
uint32_t ix_find_projects(ix_index* ix, uint32_t* out_ids, uint8_t* out_kind, uint32_t max);
// Dziecko o nazwie (bez rozrozniania wielkosci liter) albo IX_NONE.
uint32_t ix_find_child(ix_index* ix, uint32_t parent, const char* name, uint32_t len);
// Usuwa wpis razem z poddrzewem (oznacza jako usuniete, odpina od rodzica).
void ix_remove(ix_index* ix, uint32_t id);
void ix_rename(ix_index* ix, uint32_t id, const char* name, uint32_t len);
// Pelna sciezka (separator '\\'). Zwraca wymagana dlugosc; zapisuje min(len, cap).
uint32_t ix_path(ix_index* ix, uint32_t id, char* buf, uint32_t cap);
uint32_t ix_flags(ix_index* ix, uint32_t id);
uint32_t ix_count(ix_index* ix);
uint64_t ix_memory(ix_index* ix);
// Oddaje nadmiar zarezerwowanej pamieci (po zakonczeniu skanowania).
void ix_compact(ix_index* ix);

#define IX_FIND_ALL 0u
#define IX_FIND_DIRS 1u
#define IX_FIND_FILES 2u

// Szuka wpisow, ktorych nazwa zawiera wszystkie slowa zapytania.
// `mode`: IX_FIND_ALL / IX_FIND_DIRS / IX_FIND_FILES.
// Zapisuje do `out` najlepsze `max` identyfikatorow; `total` = liczba wszystkich trafien.
uint32_t ix_search(ix_index* ix, const char* query, uint32_t qlen, uint32_t mode,
                   uint32_t* out, uint32_t max, uint32_t* total);

// --- drzewo folderow (panel z lewej) ---
// Identyfikator wpisu dla pelnej sciezki (bez rozrozniania wielkosci liter) albo IX_NONE.
uint32_t ix_lookup(ix_index* ix, const char* path, uint32_t len);
// Podfoldery wpisu: zapisuje do `out` max `max` id; zwraca liczbe wszystkich podfolderow.
uint32_t ix_child_dirs(ix_index* ix, uint32_t id, uint32_t* out, uint32_t max);
// Sama nazwa wpisu (bez sciezki). Zwraca wymagana dlugosc.
uint32_t ix_name(ix_index* ix, uint32_t id, char* buf, uint32_t cap);

#ifdef __cplusplus
}
#endif
