# Audyt schematu bazy danych

Pełny audyt wszystkich 12 tabel w bazie danych Soroban Block Explorer.
Dla każdej tabeli: opisy kolumn, wszystkie ścieżki zapisu (INSERT/UPDATE/UPSERT)
oraz mutowalność po insercie.

Wygenerowano: 2026-04-15

---

## Spis treści

1. [ledgers](#ledgers)
2. [transactions](#transactions)
3. [operations](#operations)
4. [soroban_contracts](#soroban_contracts)
5. [soroban_events](#soroban_events)
6. [soroban_invocations](#soroban_invocations)
7. [accounts](#accounts)
8. [tokens](#tokens)
9. [nfts](#nfts)
10. [liquidity_pools](#liquidity_pools)
11. [liquidity_pool_snapshots](#liquidity_pool_snapshots)
12. [wasm_interface_metadata](#wasm_interface_metadata)

---

## `ledgers`

### Opis

Przechowuje jeden wiersz na każdy zaindeksowany ledger (blok) Stellar. Służy jako tabela nadrzędna dla `transactions` (FK na `ledgers.sequence`) i jest referencjonowana przez inne tabele (`soroban_contracts`, `accounts`, `liquidity_pools`) do zakotwiczenia czasowego.

### Kolumny

| Kolumna             | Typ                           | Opis                                                                    |
| ------------------- | ----------------------------- | ----------------------------------------------------------------------- |
| `sequence`          | `BIGINT PRIMARY KEY`          | Numer sekwencji ledgera. Unikalny, monotonically rosnący identyfikator. |
| `hash`              | `VARCHAR(64) NOT NULL UNIQUE` | Hash SHA-256 `LedgerHeaderHistoryEntry` XDR, zakodowany hex.            |
| `closed_at`         | `TIMESTAMPTZ NOT NULL`        | Znacznik czasu zamknięcia ledgera przez sieć (czas konsensusu).         |
| `protocol_version`  | `INTEGER NOT NULL`            | Wersja protokołu Stellar obowiązująca w tym ledgerze.                   |
| `transaction_count` | `INTEGER NOT NULL`            | Liczba transakcji w tym ledgerze.                                       |
| `base_fee`          | `BIGINT NOT NULL`             | Bazowa opłata sieciowa w stroopach (1 stroop = 0.0000001 XLM).          |

### Indeksy

| Indeks          | Kolumny          |
| --------------- | ---------------- |
| Klucz główny    | `sequence`       |
| Unique          | `hash`           |
| `idx_closed_at` | `closed_at DESC` |

### Ścieżki zapisu

| #   | Funkcja         | Plik:Linia                           | SQL                                            | Wyzwalacz                                                          | Kolumny            |
| --- | --------------- | ------------------------------------ | ---------------------------------------------- | ------------------------------------------------------------------ | ------------------ |
| 1   | `insert_ledger` | `crates/db/src/persistence.rs:22-42` | `INSERT ... ON CONFLICT (sequence) DO NOTHING` | `persist_ledger()` krok 1, wywoływany per ledger z handlera Lambda | Wszystkie 6 kolumn |

### Mutowalność po insercie

**W pełni niemutowalny.** `ON CONFLICT DO NOTHING` — żadne kolumny nie są nigdy aktualizowane. Brak instrukcji UPDATE dla tej tabeli.

---

## `transactions`

### Opis

Przechowuje jeden wiersz na każdą transakcję Stellar. Zawiera pełną kopertę transakcji, wynik i metadane zarówno w kolumnach strukturalnych, jak i surowych blobach XDR. Klucz główny to surogatowy `BIGSERIAL` id z ograniczeniem unikalności na hashu transakcji.

### Kolumny

| Kolumna           | Typ                           | Opis                                                                                                                                  |
| ----------------- | ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `id`              | `BIGSERIAL PRIMARY KEY`       | Auto-inkrementujący klucz surogatowy. Cel FK dla `operations`, `soroban_events`, `soroban_invocations`.                               |
| `hash`            | `VARCHAR(64) NOT NULL UNIQUE` | Hash SHA-256 TransactionEnvelope, zakodowany hex. Klucz deduplikacji.                                                                 |
| `ledger_sequence` | `BIGINT NOT NULL`             | Sekwencja ledgera nadrzędnego. FK do `ledgers(sequence)` (bez CASCADE — usunięcie ledgera jest blokowane dopóki istnieją transakcje). |
| `source_account`  | `VARCHAR(56) NOT NULL`        | Konto źródłowe transakcji (adres G... lub M...).                                                                                      |
| `fee_charged`     | `BIGINT NOT NULL`             | Faktycznie naliczona opłata w stroopach.                                                                                              |
| `successful`      | `BOOLEAN NOT NULL`            | Czy transakcja się powiodła.                                                                                                          |
| `result_code`     | `VARCHAR(50)`                 | Kod wyniku transakcji (np. `txSUCCESS`). Nullable.                                                                                    |
| `envelope_xdr`    | `TEXT NOT NULL`               | Pełna koperta transakcji, XDR zakodowany base64.                                                                                      |
| `result_xdr`      | `TEXT NOT NULL`               | Wynik transakcji, XDR zakodowany base64.                                                                                              |
| `result_meta_xdr` | `TEXT`                        | Metadane wyniku transakcji, XDR zakodowany base64. Nullable.                                                                          |
| `memo_type`       | `VARCHAR(20)`                 | Typ memo: `"text"`, `"id"`, `"hash"`, `"return"` lub NULL.                                                                            |
| `memo`            | `TEXT`                        | Wartość memo. Nullable.                                                                                                               |
| `created_at`      | `TIMESTAMPTZ NOT NULL`        | Znacznik czasu z czasu zamknięcia ledgera nadrzędnego.                                                                                |
| `parse_error`     | `BOOLEAN`                     | True jeśli parsowanie XDR nie powiodło się dla tej transakcji. Nullable.                                                              |
| `operation_tree`  | `JSONB`                       | Wstępnie obliczone drzewo wywołań Soroban. Wypełniane asynchronicznie. NULL dla nie-Soroban.                                          |

### Indeksy

| Indeks       | Kolumny                             |
| ------------ | ----------------------------------- |
| Klucz główny | `id`                                |
| Unique       | `hash`                              |
| `idx_source` | `(source_account, created_at DESC)` |
| `idx_ledger` | `ledger_sequence`                   |

### Ścieżki zapisu

| #   | Funkcja                        | Plik:Linia                            | SQL                                                                                   | Wyzwalacz                                               | Dotknięte kolumny                                                                                    |
| --- | ------------------------------ | ------------------------------------- | ------------------------------------------------------------------------------------- | ------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| 1   | `insert_transactions_batch`    | `crates/db/src/persistence.rs:56-129` | `INSERT ... ON CONFLICT (hash) DO UPDATE SET hash = EXCLUDED.hash RETURNING hash, id` | `persist_ledger()` krok 2                               | Wszystkie 15 kolumn przy INSERT. Przy konflikcie: no-op samo-przypisanie (aby uzyskać RETURNING id). |
| 2   | `update_operation_trees_batch` | `crates/db/src/soroban.rs:44-65`      | `UPDATE transactions SET operation_tree = ... WHERE id = ...`                         | `persist_ledger()` krok 6, tylko dla transakcji Soroban | Tylko `operation_tree`                                                                               |

### Mutowalność po insercie

Tylko **`operation_tree`** jest aktualizowany po początkowym insercie. Wszystkie inne kolumny są zapisywane raz. Klauzula ON CONFLICT jest no-opem używanym wyłącznie dla RETURNING.

### Tabele potomne

- `operations.transaction_id` → `ON DELETE CASCADE`
- `soroban_events.transaction_id` → `ON DELETE CASCADE`
- `soroban_invocations.transaction_id` → `ON DELETE CASCADE`

---

## `operations`

### Opis

Przechowuje pojedyncze operacje Stellar wyodrębnione z transakcji. Każda transakcja zawiera jedną lub więcej operacji. **Partycjonowana zakresowo po `transaction_id`** (10M ID na partycję).

### Kolumny

| Kolumna             | Typ                    | Opis                                                                          |
| ------------------- | ---------------------- | ----------------------------------------------------------------------------- |
| `id`                | `BIGSERIAL`            | Auto-generowany klucz surogatowy. Część złożonego PK `(id, transaction_id)`.  |
| `transaction_id`    | `BIGINT NOT NULL`      | FK do `transactions(id)` z CASCADE. Klucz partycji.                           |
| `application_order` | `SMALLINT NOT NULL`    | Indeks od zera tej operacji w ramach transakcji nadrzędnej.                   |
| `source_account`    | `VARCHAR(56) NOT NULL` | Konto źródłowe operacji (dziedziczone z transakcji jeśli nie nadpisane).      |
| `type`              | `VARCHAR(50) NOT NULL` | Typ operacji (np. `"INVOKE_HOST_FUNCTION"`, `"PAYMENT"`).                     |
| `details`           | `JSONB NOT NULL`       | Szczegóły operacji zależne od typu. Struktura różni się w zależności od typu. |

### Partycjonowanie

- **Metoda:** `PARTITION BY RANGE (transaction_id)`, 10M ID na partycję.
- **Początkowe:** `operations_p0` (0–10M), `operations_default`.
- **Dynamiczne:** Lambda `db-partition-mgmt` automatycznie tworzy nowe partycje gdy >80% wykorzystane.

### Indeksy i ograniczenia

| Nazwa                    | Typ          | Kolumny                               |
| ------------------------ | ------------ | ------------------------------------- |
| PK                       | Klucz główny | `(id, transaction_id)`                |
| `idx_operations_tx`      | B-tree       | `transaction_id`                      |
| `idx_operations_source`  | B-tree       | `source_account`                      |
| `idx_operations_details` | GIN          | `details`                             |
| `uq_operations_tx_order` | Unique       | `(transaction_id, application_order)` |

### Ścieżki zapisu

| #   | Funkcja                   | Plik:Linia                             | SQL                                                                      | Wyzwalacz                 | Kolumny                                                                    |
| --- | ------------------------- | -------------------------------------- | ------------------------------------------------------------------------ | ------------------------- | -------------------------------------------------------------------------- |
| 1   | `insert_operations_batch` | `crates/db/src/persistence.rs:137-178` | `INSERT ... ON CONFLICT ON CONSTRAINT uq_operations_tx_order DO NOTHING` | `persist_ledger()` krok 3 | `transaction_id`, `application_order`, `source_account`, `type`, `details` |

### Mutowalność po insercie

**W pełni niemutowalny.** `ON CONFLICT DO NOTHING`. Brak instrukcji UPDATE.

---

## `soroban_contracts`

### Opis

Przechowuje jeden wiersz na każdy wdrożony smart kontrakt Soroban. Rejestruje tożsamość kontraktu, hash WASM, wdrażającego, klasyfikację (`contract_type`), flagę SAC oraz akumulujące metadane JSONB z sygnaturami funkcji.

### Kolumny

| Kolumna              | Typ                                   | Opis                                                                                                 |
| -------------------- | ------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `contract_id`        | `VARCHAR(56) PRIMARY KEY`             | Adres kontraktu Soroban (prefiks C).                                                                 |
| `wasm_hash`          | `VARCHAR(64)`                         | Hash hex SHA-256 bytecodu WASM. Nullable (wiersze-stuby mogą istnieć przed wdrożeniem). Indeksowany. |
| `deployer_account`   | `VARCHAR(56)`                         | Konto które wdrożyło kontrakt. Nullable.                                                             |
| `deployed_at_ledger` | `BIGINT REFERENCES ledgers(sequence)` | Ledger w którym kontrakt został wdrożony. Nullable.                                                  |
| `contract_type`      | `VARCHAR(50)`                         | Klasyfikacja: `"token"`, `"dex"`, `"lending"`, `"nft"`, `"other"`. Indeksowany. Nullable.            |
| `is_sac`             | `BOOLEAN NOT NULL DEFAULT FALSE`      | Czy to jest Stellar Asset Contract. Lepki TRUE (raz ustawiony, nigdy nie wraca do FALSE).            |
| `metadata`           | `JSONB`                               | Akumulujący JSON: sygnatury funkcji, rozmiar WASM itp. Łączony operatorem `\|\|` przy każdym upsert. |
| `search_vector`      | `TSVECTOR GENERATED`                  | Wyszukiwanie pełnotekstowe po `metadata->>'name'`. Indeks GIN. Tylko w bazie.                        |

### Ścieżki zapisu

| #   | Funkcja                                   | Plik:Linia                         | SQL                                                                               | Wyzwalacz                                                                                  | Dotknięte kolumny                                                                                                                                                                                                                                                                                                                            |
| --- | ----------------------------------------- | ---------------------------------- | --------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `ensure_contracts_exist_batch`            | `crates/db/src/soroban.rs:22-40`   | `INSERT (contract_id) ON CONFLICT DO NOTHING`                                     | `persist_ledger()` krok 3b — przed insertem do tabel potomnych (events, invocations, nfts) | Tylko `contract_id` (wiersz-stub)                                                                                                                                                                                                                                                                                                            |
| 2   | `upsert_contract_deployments_batch`       | `crates/db/src/soroban.rs:69-142`  | `INSERT ... ON CONFLICT (contract_id) DO UPDATE SET ...`                          | `persist_ledger()` krok 7                                                                  | Wszystkie kolumny przy INSERT. Przy konflikcie: `wasm_hash`, `deployer_account`, `deployed_at_ledger`, `contract_type` (COALESCE — pierwszy zapis wygrywa); `is_sac` (OR — lepki TRUE); `metadata` (merge `\|\|`). Dodatkowo wykonuje osadzony UPDATE z JOINem do `wasm_interface_metadata` aby zastosować zeskładowane metadane interfejsu. |
| 3   | `update_contract_interfaces_by_wasm_hash` | `crates/db/src/soroban.rs:175-192` | `UPDATE ... SET metadata = COALESCE(metadata, '{}') \|\| $1 WHERE wasm_hash = $2` | `persist_ledger()` krok 8, per upload WASM                                                 | Tylko `metadata`                                                                                                                                                                                                                                                                                                                             |

### Mutowalność po insercie

| Kolumna              | Mutowalna?             | Mechanizm                                                           |
| -------------------- | ---------------------- | ------------------------------------------------------------------- |
| `contract_id`        | Nie                    | Klucz główny                                                        |
| `wasm_hash`          | Tylko NULL → wartość   | `COALESCE(existing, new)` — pierwszy zapis wygrywa                  |
| `deployer_account`   | Tylko NULL → wartość   | `COALESCE(existing, new)`                                           |
| `deployed_at_ledger` | Tylko NULL → wartość   | `COALESCE(existing, new)`                                           |
| `contract_type`      | Tylko NULL → wartość   | `COALESCE(existing, new)` — **nigdy nie nadpisywany po ustawieniu** |
| `is_sac`             | Tylko FALSE → TRUE     | Logika OR — lepki true                                              |
| `metadata`           | Tak, zawsze dołączalny | `existing \|\| new` merge JSON                                      |

---

## `soroban_events`

### Opis

Przechowuje zdarzenia smart kontraktów Soroban emitowane podczas wykonywania transakcji. Każdy wiersz to pojedyncze zdarzenie. **Partycjonowana zakresowo po `created_at`** (miesięcznie).

### Kolumny

| Kolumna           | Typ                           | Opis                                                                                   |
| ----------------- | ----------------------------- | -------------------------------------------------------------------------------------- |
| `id`              | `BIGSERIAL`                   | Klucz surogatowy. Część złożonego PK `(id, created_at)`.                               |
| `transaction_id`  | `BIGINT NOT NULL`             | FK do `transactions(id)` z CASCADE.                                                    |
| `contract_id`     | `VARCHAR(56)`                 | FK do `soroban_contracts` (bez CASCADE). NULL dla zdarzeń systemowych.                 |
| `event_type`      | `VARCHAR(20) NOT NULL`        | `"contract"`, `"system"` lub `"diagnostic"`.                                           |
| `topics`          | `JSONB NOT NULL`              | Zdekodowane wartości ScVal tematów jako tablica JSON (część indeksowalna/filtrowalna). |
| `data`            | `JSONB NOT NULL`              | Zdekodowany ładunek danych zdarzenia ScVal jako JSON.                                  |
| `event_index`     | `SMALLINT NOT NULL DEFAULT 0` | Indeks od zera w ramach transakcji nadrzędnej. Klucz deduplikacji.                     |
| `ledger_sequence` | `BIGINT NOT NULL`             | Sekwencja ledgera transakcji nadrzędnej.                                               |
| `created_at`      | `TIMESTAMPTZ NOT NULL`        | Znacznik czasu z czasu zamknięcia ledgera. Klucz partycji.                             |

### Partycjonowanie

Miesięczne: `soroban_events_y{YYYY}m{MM}`, plus default. Zarządzane automatycznie przez `db-partition-mgmt`.

### Indeksy i ograniczenia

| Nazwa                 | Kolumny                                            |
| --------------------- | -------------------------------------------------- |
| `idx_events_contract` | `(contract_id, created_at DESC)`                   |
| `idx_events_topics`   | GIN `(topics)`                                     |
| `idx_events_tx`       | `(transaction_id)`                                 |
| `uq_events_tx_index`  | Unique `(transaction_id, event_index, created_at)` |

### Ścieżki zapisu

| #   | Funkcja               | Plik:Linia                         | SQL                                                                  | Wyzwalacz                 | Kolumny                   |
| --- | --------------------- | ---------------------------------- | -------------------------------------------------------------------- | ------------------------- | ------------------------- |
| 1   | `insert_events_batch` | `crates/db/src/persistence.rs:186` | `INSERT ... ON CONFLICT ON CONSTRAINT uq_events_tx_index DO NOTHING` | `persist_ledger()` krok 4 | Wszystkie kolumny poza id |

### Mutowalność po insercie

**W pełni niemutowalny.** Brak instrukcji UPDATE.

---

## `soroban_invocations`

### Opis

Przechowuje spłaszczone rekordy wywołań funkcji smart kontraktów Soroban (zarówno root jak i pod-wywołania). **Partycjonowana zakresowo po `created_at`** (miesięcznie).

### Kolumny

| Kolumna            | Typ                           | Opis                                                                         |
| ------------------ | ----------------------------- | ---------------------------------------------------------------------------- |
| `id`               | `BIGSERIAL`                   | Klucz surogatowy. Część złożonego PK `(id, created_at)`.                     |
| `transaction_id`   | `BIGINT NOT NULL`             | FK do `transactions(id)` z CASCADE.                                          |
| `contract_id`      | `VARCHAR(56)`                 | FK do `soroban_contracts` (bez CASCADE). NULL dla wywołań nie-kontraktowych. |
| `caller_account`   | `VARCHAR(56)`                 | Konto lub kontrakt inicjujący wywołanie. Nullable.                           |
| `function_name`    | `VARCHAR(100) NOT NULL`       | Nazwa wywołanej funkcji. Pusty string dla tworzenia kontraktu.               |
| `function_args`    | `JSONB`                       | Zdekodowane argumenty funkcji ScVal. Nullable.                               |
| `return_value`     | `JSONB`                       | Zdekodowana wartość zwrotna ScVal. NULL dla pod-wywołań.                     |
| `successful`       | `BOOLEAN NOT NULL`            | Czy wywołanie się powiodło.                                                  |
| `invocation_index` | `SMALLINT NOT NULL DEFAULT 0` | Indeks depth-first w drzewie wywołań. Klucz deduplikacji.                    |
| `ledger_sequence`  | `BIGINT NOT NULL`             | Numer sekwencji ledgera.                                                     |
| `created_at`       | `TIMESTAMPTZ NOT NULL`        | Znacznik czasu z czasu zamknięcia ledgera. Klucz partycji.                   |

### Partycjonowanie

Miesięczne: `soroban_invocations_y{YYYY}m{MM}`, plus default. Zarządzane automatycznie przez `db-partition-mgmt`.

### Indeksy i ograniczenia

| Nazwa                      | Kolumny                                                 |
| -------------------------- | ------------------------------------------------------- |
| `idx_invocations_contract` | `(contract_id, created_at DESC)`                        |
| `idx_invocations_function` | `(contract_id, function_name)`                          |
| `idx_invocations_tx`       | `(transaction_id)`                                      |
| `uq_invocations_tx_index`  | Unique `(transaction_id, invocation_index, created_at)` |

### Ścieżki zapisu

| #   | Funkcja                    | Plik:Linia                         | SQL                                                                       | Wyzwalacz                 | Kolumny                   |
| --- | -------------------------- | ---------------------------------- | ------------------------------------------------------------------------- | ------------------------- | ------------------------- |
| 1   | `insert_invocations_batch` | `crates/db/src/persistence.rs:246` | `INSERT ... ON CONFLICT ON CONSTRAINT uq_invocations_tx_index DO NOTHING` | `persist_ledger()` krok 5 | Wszystkie kolumny poza id |

### Mutowalność po insercie

**W pełni niemutowalny.** Brak instrukcji UPDATE.

---

## `accounts`

### Opis

Przechowuje ostatni zaobserwowany stan kont Stellar. Encja stanu pochodnego z watermarkiem `last_seen_ledger` który zapobiega nadpisywaniu nowszych danych starszymi. Wypełniana z `LedgerEntryChanges` (wpisy kont created/updated/restored).

### Kolumny

| Kolumna             | Typ                           | Opis                                                                                                                       |
| ------------------- | ----------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `account_id`        | `VARCHAR(56) PRIMARY KEY`     | Adres konta Stellar (G... lub M...).                                                                                       |
| `first_seen_ledger` | `BIGINT NOT NULL`             | Ledger w którym konto zostało pierwszy raz zaobserwowane. Ustawiany przy insercie, nigdy nie aktualizowany.                |
| `last_seen_ledger`  | `BIGINT NOT NULL`             | Najnowszy ledger z aktywnością. Watermark — aktualizacje stosowane tylko gdy przychodzące >= istniejące. Indeksowany DESC. |
| `sequence_number`   | `BIGINT NOT NULL`             | Numer sekwencji transakcji konta.                                                                                          |
| `balances`          | `JSONB NOT NULL DEFAULT '[]'` | Salda konta jako tablica JSON. Obecnie tylko natywny XLM: `[{"asset_type": "native", "balance": <stroops>}]`.              |
| `home_domain`       | `VARCHAR(256)`                | Domena domowa konta. Nullable.                                                                                             |

### Ścieżki zapisu

| #   | Funkcja                       | Plik:Linia                         | SQL                                                                                                         | Wyzwalacz                 | Dotknięte kolumny                                                                                                                                                     |
| --- | ----------------------------- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `upsert_account_states_batch` | `crates/db/src/soroban.rs:196-243` | `INSERT ... ON CONFLICT (account_id) DO UPDATE SET ... WHERE last_seen_ledger <= EXCLUDED.last_seen_ledger` | `persist_ledger()` krok 9 | Wszystkie przy INSERT. Przy konflikcie: `last_seen_ledger`, `sequence_number`, `balances` (nadpisywane); `home_domain` (COALESCE — tylko jeśli nowa wartość nie-NULL) |

### Mutowalność po insercie

| Kolumna             | Aktualizowana? | Mechanizm                                              |
| ------------------- | -------------- | ------------------------------------------------------ |
| `first_seen_ledger` | **Nie**        | Zachowywana z pierwszego insertu na zawsze             |
| `last_seen_ledger`  | **Tak**        | Nadpisywana (brama watermark)                          |
| `sequence_number`   | **Tak**        | Nadpisywany                                            |
| `balances`          | **Tak**        | Nadpisywane                                            |
| `home_domain`       | **Warunkowo**  | COALESCE — nadpisuje tylko jeśli nowa wartość nie-NULL |

---

## `tokens`

### Opis

Przechowuje odkryte tokeny na sieci Stellar/Soroban. Śledzi trzy typy aktywów: `classic`, `sac` (Stellar Asset Contract) i `soroban`. Obecnie **tylko tokeny SAC** są produkowane przez indexer. Tabela typu insert-or-ignore — efektywnie niemutowalna po utworzeniu wiersza.

### Kolumny

| Kolumna          | Typ                    | Opis                                                                                                      |
| ---------------- | ---------------------- | --------------------------------------------------------------------------------------------------------- |
| `id`             | `SERIAL PRIMARY KEY`   | Auto-inkrementujący klucz surogatowy.                                                                     |
| `asset_type`     | `VARCHAR(20) NOT NULL` | Klasyfikacja tokenu. CHECK: `'classic'`, `'sac'` lub `'soroban'`. Obecnie tylko `'sac'` jest produkowany. |
| `asset_code`     | `VARCHAR(12)`          | Kod klasycznego aktywa (np. `USDC`). Obecnie zawsze NULL.                                                 |
| `issuer_address` | `VARCHAR(56)`          | Emitent klasycznego aktywa. Obecnie zawsze NULL.                                                          |
| `contract_id`    | `VARCHAR(56)`          | FK do `soroban_contracts` (bez CASCADE). Ustawiany dla tokenów SAC i soroban.                             |
| `name`           | `VARCHAR(256)`         | Nazwa wyświetlana. Obecnie zawsze NULL.                                                                   |
| `total_supply`   | `NUMERIC(28, 7)`       | Całkowita podaż. Obecnie zawsze NULL.                                                                     |
| `holder_count`   | `INTEGER`              | Liczba posiadaczy. Obecnie zawsze NULL.                                                                   |
| `metadata`       | `JSONB`                | Elastyczne metadane. Obecnie zawsze NULL (nawet nie ma w liście kolumn INSERT).                           |

### Indeksy

| Nazwa                | Typ    | Kolumny                        | Warunek                                  |
| -------------------- | ------ | ------------------------------ | ---------------------------------------- |
| `idx_tokens_classic` | Unique | `(asset_code, issuer_address)` | `WHERE asset_type IN ('classic', 'sac')` |
| `idx_tokens_soroban` | Unique | `(contract_id)`                | `WHERE asset_type = 'soroban'`           |
| `idx_tokens_sac`     | Unique | `(contract_id)`                | `WHERE asset_type = 'sac'`               |
| `idx_tokens_type`    | B-tree | `(asset_type)`                 | —                                        |

### Ścieżki zapisu

| #   | Funkcja               | Plik:Linia                         | SQL                                 | Wyzwalacz                  | Kolumny                                                                                                              |
| --- | --------------------- | ---------------------------------- | ----------------------------------- | -------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| 1   | `upsert_tokens_batch` | `crates/db/src/soroban.rs:364-411` | `INSERT ... ON CONFLICT DO NOTHING` | `persist_ledger()` krok 12 | `asset_type`, `asset_code`, `issuer_address`, `contract_id`, `name`, `total_supply`, `holder_count` (BEZ `metadata`) |

### Mutowalność po insercie

**W pełni niemutowalny.** `ON CONFLICT DO NOTHING` — żadne kolumny nie są nigdy aktualizowane. `total_supply`, `holder_count` i `metadata` są zawsze NULL bez ścieżki UPDATE.

---

## `nfts`

### Opis

Przechowuje pochodny stan NFT ze zdarzeń kontraktów Soroban (mint, transfer, burn). Każdy wiersz reprezentuje unikalne NFT identyfikowane złożonym PK `(contract_id, token_id)`. Śledzi bieżącego właściciela z watermarkiem `last_seen_ledger` dla bezpieczeństwa przy współbieżnym/nieuporządkowanym przetwarzaniu.

### Kolumny

| Kolumna            | Typ                     | Opis                                                                          |
| ------------------ | ----------------------- | ----------------------------------------------------------------------------- |
| `contract_id`      | `VARCHAR(56) NOT NULL`  | FK do `soroban_contracts` (bez CASCADE). Część złożonego PK.                  |
| `token_id`         | `VARCHAR(256) NOT NULL` | Identyfikator tokenu (reprezentacja string danych ScVal). Część złożonego PK. |
| `collection_name`  | `VARCHAR(256)`          | Opcjonalna nazwa kolekcji. Obecnie zawsze NULL.                               |
| `owner_account`    | `VARCHAR(56)`           | Aktualny właściciel. Ustawiany na `to` przy mint/transfer, NULL przy burn.    |
| `name`             | `VARCHAR(256)`          | Opcjonalna nazwa wyświetlana. Obecnie zawsze NULL.                            |
| `media_url`        | `TEXT`                  | Opcjonalny URL mediów. Obecnie zawsze NULL.                                   |
| `metadata`         | `JSONB`                 | Elastyczne metadane. Obecnie zawsze NULL.                                     |
| `minted_at_ledger` | `BIGINT`                | Ledger w którym NFT zostało zmintowane. Ustawiany tylko dla zdarzeń mint.     |
| `last_seen_ledger` | `BIGINT NOT NULL`       | Watermark — chroni przed nadpisaniem nieaktualnymi danymi.                    |

### Indeksy

| Nazwa                 | Kolumny                          |
| --------------------- | -------------------------------- |
| PK                    | `(contract_id, token_id)`        |
| `idx_nfts_owner`      | `owner_account`                  |
| `idx_nfts_collection` | `(contract_id, collection_name)` |

### Ścieżki zapisu

| #   | Funkcja             | Plik:Linia                         | SQL                                                                                                                    | Wyzwalacz                                                                        | Dotknięte kolumny                                                                                                                                               |
| --- | ------------------- | ---------------------------------- | ---------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | `upsert_nfts_batch` | `crates/db/src/soroban.rs:415-473` | `INSERT ... ON CONFLICT (contract_id, token_id) DO UPDATE SET ... WHERE last_seen_ledger <= EXCLUDED.last_seen_ledger` | `persist_ledger()` ostatni krok, po in-memory merge po `(contract_id, token_id)` | Wszystkie przy INSERT. Przy konflikcie: `owner_account` (zawsze nadpisywany), `name`/`media_url`/`metadata` (COALESCE), `last_seen_ledger` (zawsze nadpisywany) |

### Mutowalność po insercie

| Kolumna            | Aktualizowana?  | Mechanizm                                           |
| ------------------ | --------------- | --------------------------------------------------- |
| `owner_account`    | **Tak, zawsze** | Bezwarunkowo ustawiana — śledzi transfery własności |
| `name`             | Warunkowo       | COALESCE — tylko jeśli nowa wartość nie-NULL        |
| `media_url`        | Warunkowo       | COALESCE — tylko jeśli nowa wartość nie-NULL        |
| `metadata`         | Warunkowo       | COALESCE — tylko jeśli nowa wartość nie-NULL        |
| `last_seen_ledger` | **Tak, zawsze** | Brama watermark                                     |
| `collection_name`  | **Nie**         | Ustawiany tylko przy początkowym INSERT             |
| `minted_at_ledger` | **Nie**         | Ustawiany tylko przy początkowym INSERT             |

---

## `liquidity_pools`

### Opis

Przechowuje **bieżący stan** każdej puli płynności Stellar (AMM). Niepartycjonowana tabela encji. Używa `last_updated_ledger` jako monotoniczny watermark zapobiegający nadpisywaniu nowszego stanu przez powtórzenia nieuporządkowane.

### Kolumny

| Kolumna               | Typ                       | Opis                                                      |
| --------------------- | ------------------------- | --------------------------------------------------------- |
| `pool_id`             | `VARCHAR(64) PRIMARY KEY` | Identyfikator hasha puli (64-znakowy hex).                |
| `asset_a`             | `JSONB NOT NULL`          | Deskryptor pierwszego aktywa rezerwy (kod, emitent, typ). |
| `asset_b`             | `JSONB NOT NULL`          | Deskryptor drugiego aktywa rezerwy.                       |
| `fee_bps`             | `INTEGER NOT NULL`        | Opłata handlowa w punktach bazowych (np. 30 = 0.30%).     |
| `reserves`            | `JSONB NOT NULL`          | Bieżące rezerwy obu aktywów.                              |
| `total_shares`        | `NUMERIC NOT NULL`        | Całkowita liczba wyemitowanych tokenów udziałowych puli.  |
| `tvl`                 | `NUMERIC`                 | Total value locked. Nullable.                             |
| `created_at_ledger`   | `BIGINT NOT NULL`         | Ledger w którym pula została utworzona.                   |
| `last_updated_ledger` | `BIGINT NOT NULL`         | Najnowszy ledger ze zmianą stanu. Watermark.              |

### Ścieżki zapisu

| #   | Funkcja                        | Plik:Linia                     | SQL                                                                                                            | Wyzwalacz                  | Dotknięte kolumny                                                                                      |
| --- | ------------------------------ | ------------------------------ | -------------------------------------------------------------------------------------------------------------- | -------------------------- | ------------------------------------------------------------------------------------------------------ |
| 1   | `upsert_liquidity_pools_batch` | `crates/db/src/soroban.rs:248` | `INSERT ... ON CONFLICT (pool_id) DO UPDATE SET ... WHERE last_updated_ledger <= EXCLUDED.last_updated_ledger` | `persist_ledger()` krok 10 | Wszystkie przy INSERT. Przy konflikcie: tylko `reserves`, `total_shares`, `tvl`, `last_updated_ledger` |

### Mutowalność po insercie

| Kolumna                                                         | Aktualizowana?                     |
| --------------------------------------------------------------- | ---------------------------------- |
| `reserves`                                                      | **Tak**                            |
| `total_shares`                                                  | **Tak**                            |
| `tvl`                                                           | **Tak**                            |
| `last_updated_ledger`                                           | **Tak**                            |
| `pool_id`, `asset_a`, `asset_b`, `fee_bps`, `created_at_ledger` | **Nie** — niemutowalne po insercie |

---

## `liquidity_pool_snapshots`

### Opis

Tabela szeregu czasowego tylko do dopisywania, rejestrująca migawki stanu puli w każdym ledgerze gdzie nastąpiła zmiana. **Partycjonowana zakresowo po `created_at`** (miesięcznie).

### Kolumny

| Kolumna           | Typ                    | Opis                                                     |
| ----------------- | ---------------------- | -------------------------------------------------------- |
| `id`              | `BIGSERIAL`            | Klucz surogatowy. Część złożonego PK `(id, created_at)`. |
| `pool_id`         | `VARCHAR(64) NOT NULL` | FK do `liquidity_pools`.                                 |
| `ledger_sequence` | `BIGINT NOT NULL`      | Sekwencja ledgera w momencie migawki.                    |
| `created_at`      | `TIMESTAMPTZ NOT NULL` | Znacznik czasu migawki. Klucz partycji.                  |
| `reserves`        | `JSONB NOT NULL`       | Rezerwy puli w tym momencie.                             |
| `total_shares`    | `NUMERIC NOT NULL`     | Całkowite udziały puli w momencie migawki.               |
| `tvl`             | `NUMERIC`              | TVL w momencie migawki. Nullable.                        |
| `volume`          | `NUMERIC`              | Wolumen handlowy w okresie migawki. Nullable.            |
| `fee_revenue`     | `NUMERIC`              | Przychód z opłat w okresie migawki. Nullable.            |

### Partycjonowanie

Miesięczne: `liquidity_pool_snapshots_y{YYYY}m{MM}`, plus default. Zarządzane automatycznie przez `db-partition-mgmt`.

### Ścieżki zapisu

| #   | Funkcja                                 | Plik:Linia                     | SQL                                                                        | Wyzwalacz                  | Kolumny                   |
| --- | --------------------------------------- | ------------------------------ | -------------------------------------------------------------------------- | -------------------------- | ------------------------- |
| 1   | `insert_liquidity_pool_snapshots_batch` | `crates/db/src/soroban.rs:310` | `INSERT ... ON CONFLICT (pool_id, ledger_sequence, created_at) DO NOTHING` | `persist_ledger()` krok 11 | Wszystkie kolumny poza id |

### Mutowalność po insercie

**W pełni niemutowalny.** `ON CONFLICT DO NOTHING`. Ściśle tylko do dopisywania.

---

## `wasm_interface_metadata`

### Opis

Permanentna tabela pośrednia przechowująca metadane interfejsu WASM (sygnatury funkcji, rozmiar bytecodu) kluczone po `wasm_hash`. Rozwiązuje **wzorzec 2-ledgerowego wdrożenia** Soroban: WASM jest uploadowany w ledgerze A (produkując dane interfejsu), ale kontrakt jest wdrażany w ledgerze B. Ta tabela wypełnia lukę — metadane są składowane w momencie uploadu i stosowane gdy upsert wdrożenia kontraktu jest wykonywany.

### Kolumny

| Kolumna     | Typ                       | Opis                                                                                        |
| ----------- | ------------------------- | ------------------------------------------------------------------------------------------- |
| `wasm_hash` | `VARCHAR(64) PRIMARY KEY` | Zakodowany hex hash SHA-256 bytecodu WASM. Klucz naturalny (WASM jest niezmienny on-chain). |
| `metadata`  | `JSONB NOT NULL`          | Zawiera `"functions"` (tablica sygnatur funkcji) i `"wasm_byte_len"` (rozmiar bytecodu).    |

### Ścieżki zapisu

| #   | Funkcja                          | Plik:Linia                         | SQL                                                                             | Wyzwalacz                                  | Dotknięte kolumny |
| --- | -------------------------------- | ---------------------------------- | ------------------------------------------------------------------------------- | ------------------------------------------ | ----------------- |
| 1   | `upsert_wasm_interface_metadata` | `crates/db/src/soroban.rs:149-166` | `INSERT ... ON CONFLICT (wasm_hash) DO UPDATE SET metadata = EXCLUDED.metadata` | `persist_ledger()` krok 8, per upload WASM | Obie kolumny      |

### Ścieżki odczytu (używane przez inne ścieżki zapisu)

- `upsert_contract_deployments_batch` (soroban.rs:128-139) wykonuje JOIN na tej tabeli aby zastosować zeskładowane metadane do `soroban_contracts.metadata` podczas wdrożenia kontraktu.

### Mutowalność po insercie

`metadata` może być nadpisana przy konflikcie, ale jest to idempotentne (ten sam hash WASM = ten sam interfejs). `wasm_hash` jest PK i nigdy się nie zmienia.

---

## Podsumowanie: Matryca mutowalności

| Tabela                     |                 Niemutowalna                  |     Chroniona watermarkiem      |          Dołączalna           |
| -------------------------- | :-------------------------------------------: | :-----------------------------: | :---------------------------: |
| `ledgers`                  |                    **tak**                    |                                 |                               |
| `transactions`             | w większości (`operation_tree` aktualizowany) |                                 |                               |
| `operations`               |                    **tak**                    |                                 |                               |
| `soroban_contracts`        |                                               |                                 | `metadata` przez merge `\|\|` |
| `soroban_events`           |                    **tak**                    |                                 |                               |
| `soroban_invocations`      |                    **tak**                    |                                 |                               |
| `accounts`                 |                                               |  **tak** (`last_seen_ledger`)   |                               |
| `tokens`                   |                    **tak**                    |                                 |                               |
| `nfts`                     |                                               |  **tak** (`last_seen_ledger`)   |                               |
| `liquidity_pools`          |                                               | **tak** (`last_updated_ledger`) |                               |
| `liquidity_pool_snapshots` |                    **tak**                    |                                 |                               |
| `wasm_interface_metadata`  |           idempotentne nadpisywanie           |                                 |                               |

## Kolejność pipeline'u persystencji

| Krok | Funkcja                                                                      | Tabela/tabele                                         |
| ---- | ---------------------------------------------------------------------------- | ----------------------------------------------------- |
| 1    | `insert_ledger`                                                              | `ledgers`                                             |
| 2    | `insert_transactions_batch`                                                  | `transactions`                                        |
| 3    | `insert_operations_batch`                                                    | `operations`                                          |
| 3b   | `ensure_contracts_exist_batch`                                               | `soroban_contracts` (wiersze-stuby dla spełnienia FK) |
| 4    | `insert_events_batch`                                                        | `soroban_events`                                      |
| 5    | `insert_invocations_batch`                                                   | `soroban_invocations`                                 |
| 6    | `update_operation_trees_batch`                                               | `transactions` (operation_tree)                       |
| 7    | `upsert_contract_deployments_batch`                                          | `soroban_contracts`                                   |
| 8    | `upsert_wasm_interface_metadata` + `update_contract_interfaces_by_wasm_hash` | `wasm_interface_metadata` + `soroban_contracts`       |
| 9    | `upsert_account_states_batch`                                                | `accounts`                                            |
| 10   | `upsert_liquidity_pools_batch`                                               | `liquidity_pools`                                     |
| 11   | `insert_liquidity_pool_snapshots_batch`                                      | `liquidity_pool_snapshots`                            |
| 12   | `upsert_tokens_batch`                                                        | `tokens`                                              |
| 13   | `upsert_nfts_batch`                                                          | `nfts`                                                |
