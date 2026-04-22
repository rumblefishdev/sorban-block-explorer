---
title: 'Assets vs Tokens w schemacie block explorera'
type: research
status: mature
spawns:
  - ../README.md
tags: [schema, naming, stellar-taxonomy, tokens, assets]
links:
  - https://developers.stellar.org/docs/tokens/anatomy-of-an-asset
  - https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md
  - https://github.com/stellar/stellar-protocol/blob/master/core/cap-0046-06.md
history:
  - date: '2026-04-22'
    status: mature
    who: stkrolikiewicz
    note: >
      Research note drafted earlier as freestanding `docs/assets-vs-tokens-
      taxonomy-note.md`. Moved into task 0154's notes directory when the
      task was created, so the task-to-research lineage is preserved in
      place.
---

# Notatka: Assets vs Tokens w schemacie block explorera

> Dokument podsumowujący wątek dyskusji o nazewnictwie `tokens` / `assets` w
> naszej tabeli i jej zgodności z oficjalną taksonomią Stellara. Pokrywa też
> relację z tabelą `soroban_contracts` i katalog możliwych fungible assetów.
> Nie jest to ADR — to research note do wewnętrznej dyskusji przed podjęciem
> decyzji.

## TL;DR

W oficjalnej taksonomii Stellara "Stellar Assets" i "Contract Tokens" to **dwie
równorzędne kategorie**, nie synonimy. Nasza tabela `tokens` realnie trzyma
obie (`native`, `classic`, `sac`, `soroban`), czyli jest to _de facto_ tabela
`assets` nazwana po Soroban-first iteracji projektu.

Dodatkowo mamy tabelę `soroban_contracts` trzymającą deployed contracts, do
której tokens linkują przez FK. Słowo "token" u nas robi **dwie różne robocze
rzeczy**: (a) klasyfikuje _rolę kontraktu_ (`contract_type='token'`), (b) jest
nazwą tabeli z assetami. Po rename na `assets` ta niejednoznaczność znika — i
schemat odzwierciedla dokładnie Stellar'owe rozróżnienie: _kontrakt jest
tokenem (rola), reprezentuje asset (wartość)_.

Technical Design deklaruje "Soroban-first" ale explicite wymaga pełnego supportu
classic, a aktualna schema to odzwierciedla. Nazwa tabeli jest artefaktem
wcześniejszej iteracji, nie świadomą decyzją — żaden ADR jej nie uzasadnia.
Decyzję (zostawić vs rename) warto podjąć świadomie i zapisać.

---

## 1. Punkt wyjścia

Teza od jednego z developerów: "na Stellarze nie ma tokenów, są tylko assety".
Pytanie, czy to uzasadnia rename tabeli.

**Krótka odpowiedź**: ani "są tylko assety", ani "są tylko tokeny" nie jest
prawdą w pełni. Stellar ma trzy równorzędne kategorie i słowa są używane
precyzyjnie.

---

## 2. Oficjalna taksonomia Stellara

Strona [Anatomy of an Asset](https://developers.stellar.org/docs/tokens/anatomy-of-an-asset)
definiuje trzy modele tokenizacji jako **równorzędne kategorie**:

**1. Stellar Assets (with built-in SAC)** — emitowane przez konta Stellar
(`G...`). Identyfikowane parą `(asset_code, issuer)`. Stan w trustlinach. Każdy
taki asset ma deterministyczny `C...` adres dla SAC (Stellar Asset Contract),
który wystarczy zdeployować, żeby używać go w Soroban.

**2. SEP-41 Contract Tokens (Soroban-native)** — wdrażane jako WASM contract,
identyfikowane adresem `C...`. Balance w contract data entries. Spec:
[SEP-41 Token Interface](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md).

**3. ERC-3643 / SEP-57 (T-REX) Tokens** — rozszerzenie SEP-41 o compliance
(KYC, role). Ta sama tożsamość `C...` co SEP-41.

Kluczowe: **Stellar sam nazywa kategorię 1 "Assets" a kategorie 2 i 3
"Tokens"**. Nie są to synonimy. Słowo "token" w Stellar-speak ma konkretne
znaczenie: contract-based byt implementujący SEP-41 Token Interface.

Potwierdzają to inne oficjalne źródła:

- [SEP-41 Token Interface](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md) — sama specka nazywa się "Token Interface"
- [CAP-46-6 Built-in Token Contract in Soroban](https://github.com/stellar/stellar-protocol/blob/master/core/cap-0046-06.md) — core proposal mówi "Token Contract"
- [Stellar Asset Contract (SAC)](https://developers.stellar.org/docs/tokens/stellar-asset-contract) — nazwa rozwija się _Stellar Asset Contract_, bo bierze classic asset i udostępnia go jako token w Soroban. Kierunek mostu: asset → token
- [Create Contract Tokens on Stellar](https://developers.stellar.org/docs/tokens/token-interface) — docs Stellara konsekwentnie używają "Contract Tokens" dla Soroban-side

Klient w SDK to `soroban_sdk::token::TokenClient` i `token::StellarAssetClient`.
Nawet dla classic assetu wywoływanego przez SAC, client żyje w module `token::`.

---

## 3. Co dostarcza Galexie

Galexie ([Stellar Docs](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie))
eksportuje natywny format stellar-core — `LedgerCloseMeta` w XDR. Zawiera on
**kompletny stan ledgera**, classic i Soroban razem:

- wszystkie operacje classic (`Payment`, `ChangeTrust`, `ManageSellOffer`,
  `PathPaymentStrictSend`, `CreateClaimableBalance`, `AllowTrust`, `SetOptions`)
- wszystkie `LedgerEntryChanges` (accounty, trustlines, offery, claimable
  balances, liquidity pool shares)
- operacje Soroban (`InvokeHostFunction`, `ExtendFootprintTtl`,
  `RestoreFootprint`)
- Soroban meta: contract events, contract data entry changes, WASM deployments
- transaction results, fees, diagnostic events

Implikacja: **classic assets per se są obecne w danych źródłowych**. USDC
Circle'a, każdy trustline, każdy classic payment między G-kontami — wszystko
tam jest.

Potwierdzenie: [stellar-core integration docs](https://github.com/stellar/stellar-core/blob/master/docs/integration.md),
[stellar-core transactions README](https://github.com/stellar/stellar-core/blob/master/src/transactions/readme.md).

---

## 4. Jakie fungible assety są możliwe w naszej aplikacji

Na poziomie schematu (CHECK constraint w `0005_tokens_nfts.sql`) mamy cztery
realnie reprezentowane klasy plus dwa przypadki brzegowe.

### 4.1 Cztery klasy z `asset_type`

**`native`** — XLM, jedyny token natywny Stellara. Brak issuera, brak
contract_id. `uidx_tokens_native` wymusza że jest dokładnie jeden taki row.

**`classic`** — credit assety emitowane przez konta `G...`, identyfikowane parą
`(asset_code, issuer)`. Dwie podkategorie na poziomie protokołu:
alphanumeric-4 (do 4 znaków: USDC, EURC, yXLM, AQUA) i alphanumeric-12 (5–12
znaków). Trzymane w trustline'ach. **Nie mają jeszcze zdeployowanego SAC.**

**`sac`** — classic credit asset dla którego zdeployowano SAC. Ma **obie
tożsamości**: `(code, issuer)` i `contract_id`. SAC address jest
deterministyczny z `(code, issuer)`, policzalny offline. Balance w trustline'ach
(dla G) i w contract data (dla C). W praktyce każdy popularny classic asset ma
zdeployowany SAC, bo Blend/Soroswap tego wymagają.

**`soroban`** — czysto kontraktowe SEP-41 tokeny, nigdy nie istniały na
classic. Tylko `contract_id`, brak code/issuer. Balance wyłącznie w contract
data entries. Przykłady: tokeny Blend, governance tokeny, Soroswap LP share
tokeny (Soroswap wydaje własne SEP-41 reprezentujące LP shares swojego AMM —
nie używa natywnych Stellar LP).

### 4.2 Przypadki brzegowe poza tabelą `tokens`

**Classic liquidity pool shares** — Stellar ma natywne LP na poziomie protokołu
(`AssetType.ASSET_TYPE_POOL_SHARE`). Technicznie fungible assety w stellar-xdr,
ale u nas nie w `tokens` — osobna tabela `liquidity_pools` + `lp_positions`
(task 0126).

**T-REX / SEP-57 tokens** — rozszerzenie SEP-41 o compliance. Aktualnie
wpadłyby do `'soroban'` (brak dedykowanej wartości). Na razie to nie-problem,
bo ekosystem T-REX na Stellarze nascent.

### 4.3 Pułapki klasyfikacyjne które już Was dotykają

- **Task 0118 (NFT false positives)** — niektóre kontrakty emitują eventy
  `transfer` zgodne z SEP-41, ale reprezentują NFT (SEP-56 albo własne
  standardy). Detekcja "fungible vs non-fungible" na podstawie samych eventów
  zawodzi — potrzebna analiza interface'u kontraktu.
- **Task 0120 (Soroban-native non-SAC detection)** — rozróżnienie `sac` vs
  `soroban` nie wynika z eventów (oba emitują SEP-41 `transfer`), tylko z
  `is_sac` flagi na `soroban_contracts`, która pochodzi z deployment events
  (SAC ma deterministyczny `HostFunction::CreateContract` z
  `ContractIdPreimageFromAsset`). Jeśli parser tego nie wyłapuje, SAC ląduje
  jako `soroban` — false positive.
- **Niestandardowe kontrakty-tokeny** — implementują większość SEP-41, ale np.
  nie wystawiają `decimals()` albo mają `transfer` z innym schematem topiców.
  Efektywnie wymagają whitelist albo fuzzy pattern matching.

### 4.4 Tabelaryczne podsumowanie

| `asset_type` | Tożsamość                 | Trzymany w                                 | `soroban_contracts` row? | Przykład      |
| ------------ | ------------------------- | ------------------------------------------ | ------------------------ | ------------- |
| `native`     | brak                      | trustlines (G) / contract data (C via SAC) | nie (contract_id = NULL) | XLM           |
| `classic`    | `(code, issuer)`          | trustlines                                 | nie                      | yUSDC bez SAC |
| `sac`        | `(code, issuer)` + `C...` | trustlines + contract data                 | **tak (FK wymusza)**     | USDC z SAC    |
| `soroban`    | `C...`                    | contract data                              | **tak (FK wymusza)**     | Blend BLND    |

---

## 5. Co faktycznie robimy — aktualny stan write-pathu

Plik: `crates/indexer/src/handler/persist/mod.rs`. Metoda `persist_ledger`
realizuje 14-krokowy pipeline w jednej atomicznej transakcji DB (ADR 0027).

Obsługuje **zarówno classic, jak i Soroban** — jest "Soroban-first" tylko w
sensie priorytetów UX, nie scope'u danych. Zgodne z Technical Design sekcja
1.1:

> **Classic + Soroban** — Support both classic Stellar operations (payments,
> offers, path payments, etc.) and Soroban operations (invoke host function,
> contract events, token swaps).

### 5.1 Tabela `tokens` w realnej schemie

Migracja `crates/db/migrations/0005_tokens_nfts.sql`:

```sql
asset_type VARCHAR(20) NOT NULL
CHECK (asset_type IN ('native', 'classic', 'sac', 'soroban'))

CONSTRAINT ck_tokens_identity CHECK (
    (asset_type = 'native'  AND asset_code IS NULL     AND issuer_id IS NULL     AND contract_id IS NULL)
 OR (asset_type = 'classic' AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NULL)
 OR (asset_type = 'sac'     AND asset_code IS NOT NULL AND issuer_id IS NOT NULL AND contract_id IS NOT NULL)
 OR (asset_type = 'soroban' AND issuer_id IS NULL      AND contract_id IS NOT NULL)
)

CREATE UNIQUE INDEX uidx_tokens_native ON tokens ((asset_type))
    WHERE asset_type = 'native';
CREATE UNIQUE INDEX uidx_tokens_classic_asset ON tokens (asset_code, issuer_id)
    WHERE asset_type IN ('classic', 'sac');
CREATE UNIQUE INDEX uidx_tokens_soroban ON tokens (contract_id)
    WHERE asset_type IN ('soroban', 'sac');
```

SAC siedzi w obu partial unique indexach, bo ma obie tożsamości. Funkcja
`upsert_tokens` w `persist/write.rs:743-910` rozbija staged rows na cztery
klasy i dla każdej używa dedykowanego path.

### 5.2 Drift między design docem a migracjami

Design `docs/architecture/technical-design-general-overview.md` sekcja 6.7
opisuje starszy stan:

```sql
asset_type VARCHAR(10) NOT NULL CHECK (asset_type IN ('classic', 'sac', 'soroban'))
UNIQUE (asset_code, issuer_address)
UNIQUE (contract_id)
```

Różnice vs rzeczywistość:

- design: 3 wartości `asset_type`; realnie: **4** (dodany `native` dla XLM)
- design: `VARCHAR(10)`; realnie: `VARCHAR(20)`
- design: zwykłe `UNIQUE`; realnie: **partial unique indexes** per `asset_type`
- design: brak `ck_tokens_identity`; realnie: dodany

Analogiczny drift dotyczy innych tabel w sekcji 6 designu
(`transaction_hash_index`, `transaction_participants`, `wasm_interface_metadata`,
`lp_positions`, `nft_ownership`, `account_balances_current`/`history`).
Ten dokument **nie proponuje** aktualizacji designu — tylko odnotowuje że
drift istnieje.

---

## 6. Relacja z tabelą `soroban_contracts`

Tu jest kluczowy aspekt dla pytania o nazewnictwo, bo to w tej relacji leży
semantyczna kolizja słowa "token".

### 6.1 Schema

Z migracji `0002_identity_and_ledgers.sql`:

```sql
CREATE TABLE soroban_contracts (
    contract_id   VARCHAR(56) PRIMARY KEY,
    wasm_hash     BYTEA REFERENCES wasm_interface_metadata(wasm_hash),
    deployer_id   BIGINT REFERENCES accounts(id),
    deployed_at_ledger BIGINT,
    contract_type VARCHAR(50),      -- 'token', 'nft', 'dex', 'lending', 'other'
    is_sac        BOOLEAN NOT NULL DEFAULT false,
    metadata      JSONB,
    ...
);
```

I FK z tokens: `contract_id VARCHAR(56) REFERENCES soroban_contracts(contract_id)`.

### 6.2 Co to realnie mapuje

To jest **czyste, 1-do-1 odwzorowanie Stellar'owego rozróżnienia**:

- `soroban_contracts` = deployed contracts (wszystko z `C...`)
- `soroban_contracts.contract_type = 'token'` = "ten kontrakt implementuje
  SEP-41 Token Interface"
- `soroban_contracts.is_sac = true` = "ten kontrakt to SAC dla jakiegoś
  classic assetu"
- `tokens.contract_id → soroban_contracts` = "oto asset który ten
  token-kontrakt reprezentuje"

Dokładnie ten podział: **token = interface kontraktu, asset = jednostka
wartości**. Schemat _już to rozróżnia strukturalnie_.

### 6.3 Wymuszenie integralności

`ck_tokens_identity` explicite wymusza kiedy `contract_id` musi być NOT NULL:

- `native`, `classic` → `contract_id IS NULL` (brak kontraktu)
- `sac`, `soroban` → `contract_id IS NOT NULL` + FK do `soroban_contracts`

Oznacza to że **każdy `sac`/`soroban` w `tokens` wymaga odpowiadającego rowa w
`soroban_contracts`, strukturalnie**. Baza tego pilnuje.

Analogicznie NFTs: `nfts.contract_id VARCHAR(56) NOT NULL REFERENCES soroban_contracts(contract_id)`.

### 6.4 Co dodatkowo trzyma `soroban_contracts` a `tokens` nie

Dla każdego `sac`/`soroban` rekordu w `tokens`, odpowiednik w
`soroban_contracts` trzyma:

- `wasm_hash` — FK do `wasm_interface_metadata`, czyli implementacja (wszystkie
  SAC mają ten sam stub WASM, Soroban-native każdy własny)
- `deployer_id`, `deployed_at_ledger`, `wasm_uploaded_at_ledger`
- `is_sac` — kanoniczna flaga SAC vs non-SAC
- `contract_type` — classyfikacja roli kontraktu
- `metadata` JSONB — interface signatures (ADR 0023)
- `search_vector` — GIN index do wyszukiwania

### 6.5 Przykład: USDC z SAC na mainnet

1. Row w `accounts` — issuer Circle'a (`GA5ZSEJY...KZVN`)
2. Row w `wasm_interface_metadata` — stub WASM SAC-a (wspólny dla wszystkich SAC)
3. Row w `soroban_contracts`: `contract_id = CCW6...MI75`, `is_sac = true`,
   `contract_type = 'token'`, FK → wasm_interface_metadata
4. Row w `tokens`: `asset_type = 'sac'`, `asset_code = 'USDC'`,
   `issuer_id` → accounts row Circle'a, `contract_id = CCW6...MI75` → FK do
   soroban_contracts

Dla Blend BLND (Soroban-native): row w `soroban_contracts` z `is_sac = false`,
własny `wasm_hash`, `contract_type = 'token'`; row w `tokens` z `asset_type =
'soroban'`, tylko `contract_id`, bez code/issuer.

### 6.6 Edge case: XLM

XLM ma zdeployowany SAC na mainnet, aktywnie używany przez DeFi
(`CAS3J...YHXP`). W naszym schemacie `ck_tokens_identity` wymusza dla `native`
że `contract_id IS NULL` — więc row XLM w `tokens` **nie linkuje** do SAC-a
XLM-a. Jeśli XLM SAC zostanie wykryty przez parser przy detect contracts, może
wylądować w `soroban_contracts` jako osobny row, ale `tokens.native` o nim nie
wie.

Dziura w data modelu: `asset_type = 'sac'` wymaga `issuer_id IS NOT NULL`, a
XLM issuera nie ma. Więc XLM SAC nie może być reprezentowany jako `sac`.
Konsekwencja: "pokaż wszystkie contract events dla XLM" — brak ścieżki
JOIN-owej.

Warto zweryfikować w parserze jak ta ścieżka jest obsługiwana (albo świadomie
udokumentować jako known limitation).

---

## 7. Sedno: semantyczna kolizja słowa "token"

Fakty zebrane wyżej składają się w konkretną obserwację:

Słowo "token" robi u nas **dwie różne robocze rzeczy w dwóch tabelach**:

1. W `soroban_contracts.contract_type = 'token'` → klasyfikuje _rolę
   kontraktu_ jako SEP-41 Token Interface. "Token" = typ interface'u.
2. W nazwie tabeli `tokens` → trzyma _jednostki wartości_ (fungible), w tym
   classic assety które nie mają żadnego kontraktu ani SEP-41.

To jest niejednoznaczność wbudowana w nazewnictwo. Przykładowy problem w
rozmowie zespołu: "ten token jest w tabeli tokens" — o którym tokenie mówimy?
O kontrakcie z `contract_type='token'`, czy o wpisie w `tokens`? Pytanie
nietrywialne, bo classic assety w `tokens` nie mają odpowiednika w
`soroban_contracts`.

### 7.1 Argumenty za `tokens` (status quo)

- Nic nie trzeba migrować
- Słowo wygodne w mowie zespołu — wszyscy wiedzą o co chodzi
- W Soroban-world (SEP-41, SDK) "token" jest naturalne
- Wewnętrzna konwencja ponad Stellar jargon

### 7.2 Argumenty za `assets`

- Spójność z oficjalną taksonomią Stellara (strona "Anatomy of an asset" to
  parasol; "Stellar Assets" to jedna z kategorii w niej)
- Tabela już realnie trzyma classic+native (= "Stellar Assets" w jargonie
  Stellara), więc `tokens` jest mylące
- Nowy developer czytając schemę spodziewa się, że `tokens` = contract-based
  — a tam leży też classic XLM
- **Likwiduje kolizję z `soroban_contracts.contract_type = 'token'`** —
  "kontrakt jest tokenem (rola), reprezentuje asset (wartość)" staje się
  jednoznaczne
- Search na UI ("znajdź USDC") oczekuje jednego wyniku, i nasza schema
  właśnie tak to robi — nazwa "assets" lepiej oddaje rzeczywistość

### 7.3 Przykładowy rename (gdyby decyzja padła na tę stronę)

Tabela: `tokens` → `assets`

Wartości `asset_type`:

- `native` → zostaje
- `classic` → `classic_credit` (precyzyjniej, bo XLM też jest classic)
- `sac` → zostaje (jednoznaczne)
- `soroban` → `soroban_sep41` (precyzyjniej, zostawia miejsce na
  `soroban_trex`)

Opcjonalnie: `soroban_contracts.contract_type = 'token'` → `'sep41_token'`,
żeby było jasne że chodzi o interface'ową rolę, nie o wpis w `assets`. Obecna
'token' jest OK po rename tabeli, bo kolizja znika.

Struktura (partial uniques, `ck_tokens_identity`, FK) zostaje bez zmian.

Zmiany w kodzie: rename tabeli, rename kolumny w UI/API jeśli chcemy spójności
end-to-end, aktualizacja queries w `write.rs` i w axum endpointach. Migracja
PostgreSQL: `ALTER TABLE tokens RENAME TO assets` + opcjonalny remapping
wartości enum.

NFTs analogicznie: `soroban_contracts.contract_type = 'nft'` + tabela `nfts`
(instancje). Rename `nfts` **nie jest potrzebny** — tam niejednoznaczności nie
ma, bo tabela trzyma instancje (`unique (contract_id, token_id)`), nie
kontrakty.

---

## 8. Co NIE jest przedmiotem tej notatki

- Decyzja czy poprawiać drift w Technical Design docu — osobna sprawa, nie
  łączymy
- Reorganizacja podziału na `native_token` / `classic_token` / etc. w kodzie
  write-pathu — niezależne od nazwy tabeli
- Zmiana API (`/tokens/:id` vs `/assets/:id`) — można rozważać razem z renamem
  tabeli lub osobno; API może mieć inną nomenklaturę niż DB
- Rozwiązanie edge case XLM ↔ XLM SAC link — odnotowane w 6.6, ale
  niezależne od nazewnictwa

---

## 9. Inwentaryzacja miejsc do zmiany (scope renamingu)

Przegląd codebase'u + designu pokazuje gdzie słowo "token" jest używane jako
parasol obejmujący classic/native (czyli niezgodnie z Stellar-speak) i gdzie
jest legalnie używane w znaczeniu "SEP-41 contract". Poniżej pełny scope
potencjalnego renameu, z rozróżnieniem.

### 9.1 Schemat DB — centrum zmiany

Migracja `crates/db/migrations/0005_tokens_nfts.sql`:

- tabela `tokens` → `assets`
- constrainty `ck_tokens_asset_type`, `ck_tokens_identity` → `ck_assets_*`
- indeksy `uidx_tokens_native`, `uidx_tokens_classic_asset`, `uidx_tokens_soroban`,
  `idx_tokens_type`, `idx_tokens_code_trgm` → `uidx_assets_*`, `idx_assets_*`
- nazwa pliku migracji `0005_tokens_nfts.sql` — zostawić (historyczna), nowa
  migracja `ALTER TABLE tokens RENAME TO assets` + rename constraintów/indeksów

FK w innych tabelach (`operations`, `soroban_events`, `soroban_invocations`,
`nfts`) kierują do `tokens.id` — rename tabeli automatycznie pociąga za sobą
aktualizację FK bez zmian w SQL.

### 9.2 Kod Rust

**`crates/domain/src/token.rs`**:

- nazwa pliku → `asset.rs`
- `pub struct Token` → `Asset`
- docstring linia 1 ("Token domain type matching the `tokens` PostgreSQL table") — aktualizacja

**`crates/xdr-parser/src/types.rs`**:

- `pub struct ExtractedToken` → `ExtractedAsset`

**`crates/xdr-parser/src/state.rs`**:

- `pub fn detect_tokens(deployments)` → `detect_assets`
- importy `ExtractedToken`

**`crates/xdr-parser/src/classification.rs`** — logika klasyfikacji tokenów,
prawdopodobnie `TokenClassification`, `classify_token`. Wymaga dokładniejszego
audytu przy realizacji renameu.

**`crates/indexer/src/handler/persist/staging.rs`**:

- `pub(super) struct TokenRow` → `AssetRow`
- `pub token_rows: Vec<TokenRow>` → `asset_rows`
- parametr `tokens: &[ExtractedToken]` → `assets: &[ExtractedAsset]`

**`crates/indexer/src/handler/persist/write.rs:743-910`**:

- `upsert_tokens()`, `upsert_tokens_native`, `upsert_tokens_classic_like`,
  `upsert_tokens_soroban` → `upsert_assets*`

**`crates/indexer/src/handler/persist/mod.rs`**:

- parametr `tokens` w sygnaturze `persist_ledger`
- komentarz "12. tokens" w pipeline
- kolumna `tokens_ms` w `StepTimings` (linie 60, 202) — uwaga: to pokazuje się
  w logach i metrykach, rename przesłoni dashboardy Grafana/CloudWatch

**`crates/indexer/src/handler/process.rs:127`**:

- `let tokens = xdr_parser::detect_tokens(&deployments);`

**Testy `crates/indexer/tests/persist_integration.rs`**:

- helper `make_sac_token()` → `make_sac_asset()`
- importy `ExtractedToken`

### 9.3 Technical Design Overview

Tu jest najwięcej pomieszania. Trzy linijki są wręcz **smoking gun** — sam doc
przyznaje że nazwa "tokens" obejmuje coś szerszego:

> **Linia 158**: _"Balances — native XLM balance and trustline/token balances"_

Trustline balance w Stellar-speak to asset balance, nie token balance. Klasyczne
złe użycie.

> **Linia 163**: _"List of all known tokens (classic Stellar assets and Soroban token contracts)"_

Doc sam eksplicite rozpina nazwę w nawiasie — mocny sygnał, że nazwa jest za
wąska.

> **Linia 370**: _"Paginated list of tokens (classic assets + Soroban token contracts)"_

To samo, w sekcji API.

Pozostałe miejsca do aktualizacji w docu (linie przybliżone):

- 46, 58-59, 85-86 — tabele route'ów z `/tokens`, `/tokens/:id`
- 161, 165, 170, 172, 174, 177 — Tokens page / Token detail description
- 280 — ASCII diagram "Tokens" module w backend Lambda
- 368-376 — sekcja "Tokens" endpoints
- 414 — search params `type=...,token,...`
- 470 — ASCII diagram RDS listing `tokens` tabelę (do rename spójnie z DB)
- 739 — "Derived-state upserts (`accounts`, `tokens`, `nfts`, `liquidity_pools`)"
- 948, 951 — sekcja 6.7 nagłówek i `CREATE TABLE tokens`
- 1072, 1086, 1110-1111, 1208 — estimate tables i deliverables

### 9.4 API endpoints (publiczny kontrakt)

Z designu sekcja 2.3:

- `GET /tokens` → `/assets`
- `GET /tokens/:id` → `/assets/:id`
- `GET /tokens/:id/transactions` → `/assets/:id/transactions`
- query param `type=...,token,...` w `/search` — rozważyć czy "token" jako
  filter-type zostaje, czy zmienia się na "asset"

**To jest publiczny kontrakt**. Jeśli API jest pre-launch, rename bezbolesny.
Jeśli już out, trzeba versioning (`/v1/tokens` stary, `/v2/assets` nowy) albo
aliasy.

### 9.5 ADR-y — zostawiamy

Pliki historycznie używają "token" w tytułach i treści:

- `0022_schema-correction-and-token-metadata-enrichment.md`
- `0023_tokens-typed-metadata-columns.md`
- `0027_post-surrogate-schema-and-endpoint-realizability.md` (tokens w treści)

**Nie renamować**. ADR-y mają wartość historyczną. Nowy ADR z decyzją
o renamie zaktualizuje kontekst przyszłych czytelników bez przepisywania
historii.

### 9.6 Miejsca gdzie "token" jest używany **poprawnie** — nie rusz

- `soroban_contracts.contract_type = 'token'` — rola kontraktu (SEP-41)
- `nfts.token_id` — standardowa terminologia NFT (`(contract_id, token_id)` = identyfikator instancji)
- "Token Interface" / "SEP-41 Token" — oficjalny termin protokołu
- `soroban_sdk::token::TokenClient`, `token::StellarAssetClient` — nazewnictwo Rust SDK, nie nasze
- "Detect token contracts (SEP-41)" w designu linia 669 — poprawne, "token contract" = SEP-41 contract
- "Token swap" w opisach Soroban DEX — poprawne w tym kontekście

### 9.7 Podsumowanie scope'u

Pliki dotknięte: ~15–20. Zmiany w większości mechaniczne (rename struct /
funkcji / importów / kolumn). Ryzykowne miejsca: migracja DB (ALTER TABLE +
rename constraintów i indeksów w jednej transakcji) oraz API (wersjonowanie
jeśli publiczne). Pozostałe zmiany to rename-wszystko-od-razu przez
`cargo check` jako guarda.

Szacunkowy wysiłek: 1–2 dni jednego developera, w tym migracja DB z
rollbackiem, aktualizacja designu, rename w kodzie, test suite green. Większa
część to mechanika, niewiele trudnych decyzji.

---

## 10. Proponowany format decyzji

Opcja A: zostawiamy `tokens`, ale dopisujemy krótki ADR ustalający że to
parasol i że świadomie odbiegamy od Stellar'owej taksonomii (rationale: nie
chcemy migracji, wewnętrzna spójność). Bez zmian w kodzie.

Opcja B: rename na `assets`. ADR dokumentuje decyzję + migracja bazy + update
queries. Większa jednorazowa praca, potem czystsza nomenklatura, usunięta
kolizja z `soroban_contracts.contract_type = 'token'`, brak dalszych dyskusji.

Opcja C: status quo bez ADR-a. Żyjemy z driftem między naszą nazwą a Stellar
jargonem. Ryzyko: powracające pytania przy każdym nowym developerze / przy
publicznej dokumentacji API.

Osobiście preferencja: **A lub B, zależnie od tego ile kosztuje teraz
migracja**. C (cichy status quo) jest najgorszy, bo problem wraca.

---

## Źródła

### Oficjalne Stellar

- [Anatomy of an Asset — Stellar Docs](https://developers.stellar.org/docs/tokens/anatomy-of-an-asset) — strona kluczowa, taksonomia trzech modeli
- [Create Contract Tokens on Stellar — Stellar Docs](https://developers.stellar.org/docs/tokens/token-interface)
- [Stellar Asset Contract (SAC) — Stellar Docs](https://developers.stellar.org/docs/tokens/stellar-asset-contract)
- [SEP-41: Token Interface](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0041.md)
- [CAP-46-6: Built-in Token Contract in Soroban](https://github.com/stellar/stellar-protocol/blob/master/core/cap-0046-06.md)
- [soroban_sdk::token::TokenInterface — Rust SDK docs](https://docs.rs/soroban-sdk/latest/soroban_sdk/token/trait.TokenInterface.html)

### Galexie / data pipeline

- [Galexie — Stellar Docs](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie)
- [Introducing Galexie (Stellar blog)](https://stellar.org/blog/developers/introducing-galexie-efficiently-extract-and-store-stellar-data)
- [stellar-core integration.md — emits LedgerCloseMeta over pipe](https://github.com/stellar/stellar-core/blob/master/docs/integration.md)
- [stellar-core transactions README](https://github.com/stellar/stellar-core/blob/master/src/transactions/readme.md)

### Community / third-party (potwierdzające używanie obu terminów)

- [Navigating Classic Assets and Smart Contract Tokens on Soroban — Cheesecake Labs](https://cheesecakelabs.com/blog/native-tokens-vs-soroban-tokens/)
- [stellar-cli issue #934: refers to 'stellar asset contract' as 'token'](https://github.com/stellar/stellar-cli/issues/934) — przykład że sam zespół Stellara widzi mieszanie terminów jako real nuance

### Nasze pliki

- `docs/architecture/technical-design-general-overview.md` — sekcja 1.1 (Classic + Soroban goal), 4.1 (pipeline), 6.4 (soroban_contracts), 6.7 (tokens schema — wersja v1)
- `crates/indexer/src/handler/persist/mod.rs` — `persist_ledger` (14 kroków, ADR 0027)
- `crates/indexer/src/handler/persist/write.rs:743-910` — `upsert_tokens` + warianty per kind
- `crates/db/migrations/0002_identity_and_ledgers.sql:40-57` — `soroban_contracts` schema
- `crates/db/migrations/0005_tokens_nfts.sql:16-47` — `tokens` schema (4 kindy, partial uniques, `ck_tokens_identity`, FK do `soroban_contracts`)
- `lore/2-adrs/0022_schema-correction-and-token-metadata-enrichment.md`
- `lore/2-adrs/0023_tokens-typed-metadata-columns.md`
- `lore/2-adrs/0027_post-surrogate-schema-and-endpoint-realizability.md`
