# pluck

> **AI 에이전트?** 이 파일은 사람을 위한 문서입니다 — 산문, 다이어그램, 시각 자료 포함.
> 에이전트용 파일은 [`AGENT.md`](AGENT.md): 툴 스펙, 노이즈 없음, 토큰 효율 최적화.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![Warm search](https://img.shields.io/badge/warm%20search-0.06%20ms%20p50-brightgreen.svg)](benchmarks/baseline.json)
[![Session dedup](https://img.shields.io/badge/session%20dedup-40%25%20fewer%20tokens-brightgreen.svg)](benchmarks/baseline.json)

**AI 코딩 에이전트를 위한 MCP-네이티브 코드 검색기.**

`pluck`은 MCP(Model Context Protocol)를 통해 심볼 인식 기반의 코드 읽기·검색을 에이전트에게 노출하는 로컬 Rust 데몬입니다. 장시간 에이전트 세션 안에서 기본 검색 표면으로 자리잡도록 설계됐습니다: warm 검색 p50 0.06 ms, 파일 변경부터 검색 가능까지 p50 183 ms, 그리고 이미 보여준 청크는 1-토큰 플레이스홀더로 대체되어 에이전트가 같은 컨텍스트에 토큰을 다시 지불하지 않습니다.

## pluck이 강한 곳과 그렇지 않은 곳

**pluck이 sharp한 영역:**

- **데몬 상주, 서브 밀리초 warm 검색.** 호출마다 인덱스를 다시 만들지 않음.
- **세션 중복 제거.** 한 MCP 세션 안에서 에이전트가 이미 본 청크는 `[already-shown: chunk_id]` (1 토큰)으로 치환. 상태가 없는 CLI 도구는 구조적으로 따라올 수 없는 영역.
- **파일 워처.** 파일 저장 → 검색 가능까지 p50 183 ms. 루프에 `index` 명령 끼울 필요 없음.
- **MCP-네이티브.** 6개 툴 모두 handshake 단계에서 `tools/list`로 설명을 노출 — `CLAUDE.md`에 프롬프트 가이드를 손으로 박지 않아도 에이전트가 적절한 툴을 고름.

**pluck이 (아직) 약한 영역:**

- **만능 CLI.** 다른 Rust 코드 검색 도구들은 `digest` 스타일의 빌드/CI 로그 압축, 파일 단위 의존/임팩트 그래프, 탐색 추천, 더 많은 언어 지원을 이미 제공해요. pluck은 아직 없지만, v0.2.0/v0.4.0에서 추가될 예정이에요 ([`docs/ROADMAP.md`](docs/ROADMAP.md) 참고).
- **이미 심볼명을 아는 경우의 grep 대체.** 리터럴 문자열 검색은 순수 `ripgrep`이 압도적으로 빠름. `pluck.grep`은 ripgrep 패스스루라 어느 쪽을 써도 무방.

핵심 가설은 **장시간 에이전트 세션 안에서는 inner loop가 누적된다**는 것. 한 번 쓰고 끝나는 one-shot 워크플로는 cold-start CLI도 충분. pluck의 강점은 30번째 호출 이후부터 드러남.

지금 당장 가장 넓은 one-shot CLI 표면이 필요하면 이미 익숙한 broad-CLI 도구와 pluck을 함께 써도 됨. 상호 배타 아님.

## 설치

### 한 줄 (0.1.0 crates.io 배포 후)

```bash
cargo install pluck-cli pluck-mcp
pluck init --target claude        # 현재 디렉토리의 .mcp.json에 등록
# 또는:
pluck init --target codex         # ~/.codex/config.toml에 등록
```

`pluck init`은 `which`로 `pluckd` 경로를 찾아 프로젝트(또는 Codex의 경우 전역)에 등록하며 idempotent — 바이너리 위치가 바뀌었을 때 언제든 재실행 가능.

### 소스 설치 (지금 바로 가능)

```bash
git clone https://github.com/hunhee98/pluck
cd pluck
cargo install --path crates/pluck-mcp     # → pluckd
cargo install --path crates/pluck-cli     # → pluck
pluck init --target claude
```

### 검증

```bash
scripts/smoke.sh
```

여섯 가지 end-to-end 체크 (version + index + search + read + grep)로 설치가 절반만 된 상태가 아님을 확인.

## 작동 원리

pluck은 Tree-sitter로 파일을 AST 단위로 청크화합니다. 에이전트가 쿼리하면 BM25 (키워드)와 `model2vec`-스타일 정적 임베딩 ([`potion-code-16M`](https://huggingface.co/minishlab/potion-code-16M))을 RRF(reciprocal-rank fusion)로 결합해 청크를 랭킹. 런타임에 트랜스포머 추론 없음 — 인코더는 룩업 행렬이고 디스크에서 ~60 MB.

```mermaid
flowchart TD
  A[소스 파일] --> B[Tree-sitter\nAST 청크화]
  B --> C[tantivy BM25F 인덱스]
  B --> D[정적 임베딩\npotion-code-16M]
  C --> E[in-RAM 인덱스]
  D --> E
  E --> F[pluckd MCP 데몬]
  G[파일 워처\n150ms 디바운스] -->|증분 재인덱싱| E
  H[에이전트 쿼리] --> F
  F --> I[BM25 + 시맨틱 RRF]
  I --> J[12% 노이즈 컷오프]
  J --> K[세션 중복 제거]
  K --> L[랭킹된 스니펫 → 에이전트]
```

인덱스는 데몬 시작 시 다시 빌드합니다 (mmap 영구 디스크 인덱스는 SOON 로드맵, v0.1.0 아님).

### 세션 중복 제거 동작

```mermaid
sequenceDiagram
  participant A as 에이전트
  participant P as pluckd
  A->>P: search("auth token")
  P->>A: chunk A (body, 420 tok) + chunk B (body, 380 tok)
  Note over P: 세션 set: {A, B}
  A->>P: search("session expiry")
  P->>A: [already-shown: chunk A, 1 tok] + chunk C (body, 340 tok)
  Note over A,P: chunk A는 이미 컨텍스트에 있음 — pluck은 플레이스홀더 반환
```

`session_dedup` 벤치 기준 세션 전체 토큰의 약 **40%를 dedupe** ([`benchmarks/baseline.json`](benchmarks/baseline.json), `session_dedup_session_savings_pct` 참고).

## 6개의 MCP 툴

에이전트는 필요에 따라 특정 툴을 호출. Bash는 fallback이지 기본이 아님.

```mermaid
flowchart TD
  Q{무엇이 필요?} --> A[심볼명 이미 앎]
  Q --> B[개념/의도로 검색]
  Q --> C[정확한 정규식]
  Q --> D[파일 전체]
  A --> A1{얼마나?}
  A1 -->|시그니처만| peek[pluck.peek]
  A1 -->|전체 본문| symbol[pluck.symbol]
  A1 -->|호출 트리| expand[pluck.expand]
  B --> search[pluck.search]
  C --> grep[pluck.grep]
  D --> read[pluck.read]
```

| 툴 (와이어명) | 대체 대상 | 사용 시점 |
|---------------|-----------|-----------|
| `mcp__pluck__read` | `cat` | 코드 파일 읽기 (기본은 스마트 아웃라인; `raw: true`로 바이트 단위 정확) |
| `mcp__pluck__grep` | `grep` / `rg` | 키워드 검색 (모든 ripgrep 플래그 래핑) |
| `mcp__pluck__search` | — | 랭킹된 청크 검색 (BM25 + 시맨틱 RRF) |
| `mcp__pluck__symbol` | `cat` + 스크롤 | 특정 함수/클래스만 읽기 |
| `mcp__pluck__peek` | — | 시그니처 + 직접 callee만 |
| `mcp__pluck__expand` | 여러 번의 `cat` | 심볼 + N홉까지 callee |

모든 툴에 `raw` 모드 fallback이 있어서 `cat` / `grep`과 바이트 단위 동일 — pluck을 기본으로 써도 에이전트의 capability 손실 없음.

## 독립 실행 CLI (에이전트 없이)

```bash
pluck index .
pluck search "auth flow" --repo .
pluck read src/auth/login.ts        # 스마트 아웃라인
pluck read src/auth/login.ts --raw  # 바이트 단위 cat과 동일
pluck grep "TODO"                   # ripgrep 패스스루
```

## 측정치

페이지에 나오는 모든 숫자는 frozen baseline 행 또는 측정된 시나리오에 cite. 예측/희망 % 없음.

| 지표 | 값 | 출처 |
|------|----|------|
| Chunker p50 (medium repo) | 4.24 ms | `benchmarks/baseline.json` → `chunker_medium_ms_p50` |
| Indexer 처리량 (medium) | 386 files/s | `benchmarks/baseline.json` → `indexer_files_per_sec_medium` |
| Warm 검색 p50 (medium) | 0.06 ms | `benchmarks/baseline.json` → `warm_search_p50_ms_medium` |
| 파일 저장 → 검색 가능 p50 | 183 ms | `benchmarks/baseline.json` → `freshness_p50_ms_medium` |
| 세션 중복 제거 절감 | 40 % | `benchmarks/baseline.json` → `session_dedup_session_savings_pct` |
| 단일 시나리오 토큰 감소 (`fix/auth-token-expiry`, bash vs pluck) | 1248 → 931 tok (-25 %) | [`benchmarks/results/fix-auth-token-expiry-…json`](benchmarks/results/fix-auth-token-expiry-1778750775.json) |

`fix` / `refactor` / `explore` / `search` / `review` 시나리오 전반에 걸친 LLM-in-the-loop 측정은 v0.5.0 로드맵 (`real LLM-in-loop bench`). 숫자는 측정된 뒤에 공개 — 그 전에 미리 만들지 않음.

## Capability 비교

vs 순수 `cat` + `grep` / `rg` — pluck이 추가하는 것:

| Capability | `cat` + `grep` / `rg` | **pluck** |
|------------|------------------------|-----------|
| AST 단위 청크 | ✗ | ✓ |
| Hybrid BM25 + 시맨틱 랭킹 | ✗ | ✓ |
| 데몬 상주 (MCP stdio) | ✗ | ✓ |
| 증분 재인덱싱 (파일 워처) | ✗ | ✓ |
| 세션 단위 중복 제거 | ✗ | ✓ |
| `--raw` cat/grep 바이트 동등 | — | ✓ |
| 무손실 기본, 손실 모드 opt-in | — | ✓ |
| `peek` (시그니처 + 직접 callee) | ✗ | ✓ |
| 단일 파일 아웃라인 | ✗ | ✓ |
| 멀티 홉 `expand` (콜 그래프) | ✗ | ✓ |

vs 비슷한 hybrid-search CLI 도구 — 정직한 분할:

| Capability | 다른 Rust 코드 검색 CLI | **pluck** |
|------------|-----------------|-----------|
| Hybrid BM25 + 시맨틱 랭킹 | ✓ | ✓ |
| Tree-sitter AST 청크화 | ✓ | ✓ |
| 빌드/CI 로그 압축 (`digest`) | ✓ | ✗ — v0.2.0 |
| 파일 단위 의존 그래프 (`deps`/`impact`) | ✓ | partial (`expand`가 심볼 단위 callee) |
| 탐색 추천 (`plan`) | ✓ | ✗ — v0.2.0 |
| 언어 커버리지 | 11 | 5 (Rust / Py / TS / Go / JS) — v0.4.0에 12로 |
| MCP-네이티브 (handshake 툴 설명) | ✗ (CLAUDE.md 프롬프트만) | ✓ |
| 데몬 상주 warm 검색 | ✗ (호출마다 인덱스 재빌드) | ✓ — 0.06 ms p50 |
| 세션 단위 중복 제거 | ✗ | ✓ — 벤치 기준 40% 절감 |
| 워처 / 증분 | ✗ | ✓ — 183 ms p50 |
| 단일 바이너리 설치 | ✓ | ✓ — `cargo install` + `pluck init` |

정직하게 읽으면: 비슷한 도구들이 지금 더 넓은 one-shot CLI 표면 (digest, deps/impact, plan, 언어)을 가지고 있고, pluck은 더 깊은 inner-loop 아키텍처 (MCP-네이티브, 데몬 상주, 워처, 세션 인식)를 가지고 있음. 로드맵은 v0.1.0에서 표면 패리티를 추구하는 게 아니라, **아키텍처 우위 자리에서** 표면 갭을 메워가는 순서.

## 아키텍처

```mermaid
graph LR
  core[pluck-core\n인덱서 · 검색 · 청커 · 워처]
  mcp[pluck-mcp\npluckd 바이너리 · MCP 핸들러 · 세션 상태]
  cli[pluck-cli\n독립 CLI · pluck init]
  bench[pluck-bench\n벤치마크 하니스]
  mcp --> core
  cli --> core
  bench --> core
```

`pluck-core` 크레이트는 설계상 MCP에 의존하지 않음 — 그래서 나중에 Aider / OpenHands / Cursor 하니스를 pluggable하게 추가 가능 (v0.5.0, [`docs/ROADMAP.md`](docs/ROADMAP.md) 참고).

## 로드맵

[`docs/ROADMAP.md`](docs/ROADMAP.md)는 향후 1년을 버전별로 분할 (v0.1.0 ship cutline → v0.5.0 adoption + observability). 요약:

- **v0.1.0** (next): 첫 crates.io 배포, 3개 safety 가드, `pluck init`, OSS 위생. 좁은 표면, 새 툴 없음.
- **v0.2.0**: `pluck.digest`, `pluck.impact`, `pluck.deps`, `pluck.plan` — CLI 표면 갭 메우기.
- **v0.3.0**: NL recall 품질 — cascade, query expansion, 100-쿼리 라벨 세트, NDCG@10.
- **v0.4.0**: Java / C / C++ / Kotlin / Ruby / PHP / Swift 청커.
- **v0.5.0**: 채택률 카운터, A/B description 하니스, LLM-in-loop 벤치, 멀티 에이전트 하니스.

## 라이선스

MIT — [LICENSE](LICENSE) 참고.

## English readme

[`README.md`](README.md).
