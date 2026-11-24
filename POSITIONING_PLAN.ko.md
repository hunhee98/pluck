# pluck 포지션 변경 기획

- 작성: 이훈희
- 상태: 초안 (사용자 승인 전)
- 기준일: 2026-11-10
- 기준 커밋: `06e19ad` (v0.6.0)

이 문서는 로컬 기획 문서입니다. 저장소 이름 변경, 패키지 배포, push, 이슈·PR 생성은 이 문서를 승인한 뒤에 합니다.

## 1. 현재 포지션

### README 첫 화면

| 위치 | 현재 내용 |
|---|---|
| 제목 아래 한 줄 | "The MCP-native Code Retrieval Engine for AI Agents" |
| 부제 | "84-88 % fewer tokens on code reads · 71 % shorter CI logs · 0.07 ms warm search — every number gated by baseline.json" |
| 첫 문단 | 토큰 절감 84-88 %, 로그 압축 71 %, 검색 1 ms 미만을 굵게 표시 |
| 시작하기 | 에이전트에 붙여 넣는 설치 프롬프트 약 50줄, 그 뒤에 `cargo install`·`brew tap` |
| 상단 목차 | Quickstart · Why Pluck? · MCP Tools · CLI · Benchmarks |

README 첫 화면은 "엔진"과 측정값을 앞세웁니다. 도구가 어떤 불편을 없애는지보다 수치가 먼저 보입니다.

### 벤치마크 비중

| 대상 | 벤치마크 관련 분량 |
|---|---|
| `README.md` (292줄) | "Performance & Token Savings" 절 66줄(약 23 %). 표 4개, 막대그래프 이미지 1개, 다른 도구 비교표 17행. 부제와 첫 문단에도 수치 4개 |
| `README.ko.md` (281줄) | 같은 구성 |
| GitHub 저장소 설명 | "84-88% fewer read tokens, BM25F + semantic search, …" |
| Claude Code 플러그인 설명 | "~85% fewer tokens, sub-millisecond search" |
| 문서 | `docs/BENCHMARKS.md` 123줄, `benchmarks/README.md` 87줄, `benchmarks/llm-bench/README.md` 86줄 |
| CI | `nightly-benchmark.yml` 매일 17:00 UTC 실행, 결과를 `benchmark-results` 릴리스 태그에 게시. `ci.yml` 의 `bench-regression` 작업 |
| 기여 규칙 | `CONTRIBUTING.md` 가 엔진 변경마다 회귀 게이트 실행과 커밋 본문 수치 기재를 요구 |
| 로드맵 | v0.8.0 "LLM-in-loop bench", v1.0.0 "benchmark dashboard" |

### 현재 지표

| 항목 | 값 | 확인 방법 |
|---|---|---|
| GitHub 스타 | 43 | GitHub API |
| 포크 | 1 | GitHub API |
| 최근 14일 조회 | 33회 (고유 방문자 30명) | GitHub traffic API |
| crates.io `pluck-mcp` 누적 내려받기 | 147 | crates.io API |
| v0.6.0 릴리스 바이너리 내려받기 | 22 | GitHub API |
| 공식 MCP Registry 등록 | 없음 | registry.modelcontextprotocol.io 검색 |

### 설치 경로

| 경로 | 현재 동작 |
|---|---|
| `cargo install pluck-mcp pluck-cli` | 사용자 PC에서 Rust 소스를 빌드합니다. Rust 도구 체인이 필요합니다. |
| `brew tap hunhee98/pluck && brew install pluck` | 공식 Formula 가 `depends_on "rust" => :build` 로 소스를 빌드합니다. |
| GitHub 릴리스 | macOS(arm64·x86_64)와 Linux(arm64·x86_64) 사전 빌드 바이너리가 있지만 README 에 안내가 없습니다. Windows 바이너리는 없습니다. |
| 첫 실행 | 임베딩 모델 `potion-code-16M`(약 60 MB)을 Hugging Face 에서 내려받습니다. |

## 2. 바꿀 포지션

### 한 문장

> Claude Code·Codex·Cursor 사용자가 큰 저장소에서 작업할 때, 에이전트가 파일 전체를 `cat` 으로 읽는 대신 필요한 함수만 읽게 바꿔 컨텍스트 소모를 줄이는 MCP 서버입니다. 설치 명령 한 줄과 등록 명령 한 줄로 적용합니다.

| 항목 | 내용 |
|---|---|
| 대상 | Claude Code·Codex·Cursor 로 큰 저장소를 다루는 개발자 |
| 문제 | 에이전트가 파일 전체를 읽어 컨텍스트가 빨리 차고, 같은 코드를 여러 번 다시 읽음 |
| 해결 | `read` 가 함수 목록(outline)만 돌려주고, 필요한 함수 본문만 따로 읽음 |
| 적용 시간 | 설치 1줄 + 등록 1줄. "1분" 문구는 측정한 뒤에만 씁니다 (4절 참고) |

### README 첫 화면 구성안

| 순서 | 구성 요소 | 내용 |
|---|---|---|
| 1 | 한 줄 소개 | 에이전트가 파일 전체 대신 필요한 함수만 읽게 한다는 문장 1개. 수치 없음 |
| 2 | 설치 한 줄 | 사전 빌드 바이너리를 쓰는 명령 1개. 클라이언트 등록 명령 1개 |
| 3 | 사용 전후 예시 | 같은 파일을 `cat` 으로 읽은 결과와 `read` 로 읽은 결과를 나란히 표시. 터미널 녹화(GIF) 1개 |
| 4 | 지원 클라이언트 | Claude Code · Codex · Cursor · 그 밖의 MCP stdio 클라이언트 |
| 5 | 다음 문서 링크 | 도구 목록 · 설치 상세 · 벤치마크 · 아키텍처 |

README 초안(영문)은 3절 추천안 `open-codereader` 를 이름으로 썼습니다.

````markdown
# open-codereader

Your coding agent reads whole files. open-codereader makes it read only the functions it needs.

```bash
brew install hunhee98/tap/open-codereader
open-codereader init --target claude    # or: codex, cursor
```

## Before / after

Agent asks for `src/auth/login.ts`:

| Without | With open-codereader |
|---|---|
| `cat` → all 120 lines (929 tokens) | `read` → 5 signatures + small helpers (116 tokens), then only the body it needs |

![demo](assets/demo.gif)

## Works with

Claude Code · Codex · Cursor · any MCP stdio client

[Tools](docs/MCP_TOOLS.md) · [Install details](docs/AGENT_INSTALL.md) · [Benchmarks](docs/BENCHMARKS.md) · [How it works](docs/ARCHITECTURE.md)
````

- 사용 전후 예시의 929·116 토큰은 README "Eligible read-token savings" 표의 medium realistic 행 값입니다. README 에 남기는 수치는 이 한 쌍입니다.
- 설치 명령은 Homebrew Formula 를 사전 빌드 바이너리로 바꾼 뒤에 씁니다 (5절 작업 6).

### 설치 한 줄을 만들기 위한 변경

| 방법 | 내용 | 비고 |
|---|---|---|
| Homebrew 사전 빌드 Formula | Formula 가 릴리스 tar.gz 를 받도록 변경. 사용자 PC에서 Rust 빌드를 하지 않음 | 추천. 바이너리가 이미 릴리스에 있음 |
| npm 래퍼 패키지 | `npx -y open-codereader` 로 실행. `claude mcp add open-codereader -- npx -y open-codereader` 한 줄로 설치와 등록 | MCP 서버 배포에 많이 쓰는 형식. 플랫폼별 바이너리 패키지와 npm 배포 관리가 추가됨 |
| 설치 스크립트 | `curl -fsSL …/install.sh \| sh` | 스크립트 유지 관리가 추가됨 |

## 3. 이름 후보

조회일 2026-11-10. 레지스트리는 HTTP 404 응답을 "없음"으로 판정했습니다. GitHub 은 저장소 검색 API 첫 페이지에서 이름이 정확히 같은 저장소를 찾았습니다.

| 후보 | GitHub 같은 이름 저장소 | crates.io | npm | PyPI | Homebrew core | 검토 의견 |
|---|---|---|---|---|---|---|
| `open-codereader` | 없음 | 없음 | 없음 | 없음 | 없음 | 추천. `-mcp`·`-cli`·`-core` 크레이트와 npm 이름도 없음. "codereader mcp" 저장소 검색 0건 |
| `open-codesearch` | 없음 | 없음 | 없음 | 없음 | 없음 | `-mcp`·`-cli`·`-core` 이름 없음. 이름에 "codesearch" 가 든 저장소 460개, "codesearch mcp" 23개로 검색 결과에서 구분이 어려움 |
| `open-codeindex` | 없음 | 없음 | 없음 | 없음 | 없음 | `-mcp`·`-cli`·`-core` 이름 없음. 사용자 이점보다 내부 방식을 말함 |
| `open-pluck` | karan-ksrk/open-pluck (스타 0) | 없음 | 없음 | 없음 | 없음 | 기존 이름을 이어 감. 같은 이름 저장소가 ChatGPT·Claude.ai 용 Chrome 확장이라 사용자층이 겹침 |
| `open-peek` | 없음 | 없음 | 없음 | 없음 | 없음 | `peek` 은 도구 10개 중 하나의 이름이라 전체 기능을 말하지 않음 |
| `open-outline` | 없음 | 없음 | 없음 | 없음 | 없음 | Outline 은 VPN(Jigsaw)과 위키 제품 이름과 같음 |
| `open-codegraph` | 없음 | 없음 | 없음 | 없음 | 없음 | 호출 그래프 기능(`impact`·`expand`)만 드러냄. 비슷한 이름의 기존 프로젝트 수는 확인 안 함 |
| `open-repomap` | 없음 | 없음 | 없음 | 없음 | 없음 | "repo map" 은 Aider 의 기능 이름과 같음 |
| `open-codelens` | 없음 | 없음 | 없음 | 없음 | 없음 | CodeLens 는 VS Code·Visual Studio 기능 이름과 같음. 제외 |
| `open-codemap` | Alaa-Taieb/Open-Codemap (스타 2) | 없음 | 없음 | 없음 | 없음 | 같은 이름 저장소가 같은 분야(tree-sitter 기반 에이전트용 코드 색인, 2026-10-18 갱신). 제외 |

- 공식 MCP Registry 에는 위 후보 이름으로 등록된 서버가 없습니다.
- 다른 상용 제품 이름을 붙이는 `open-<제품명>` 형식은 상표 문제가 생길 수 있어 후보에서 뺐습니다.
- 상표 등록 여부는 확인하지 않았습니다.

### 추천: `open-codereader`

| 근거 | 내용 |
|---|---|
| 기능 전달 | 핵심 동작(파일을 `cat` 대신 함수 단위로 읽기)을 이름이 말함 |
| 이름 사용 가능 | GitHub·crates.io·npm·PyPI·Homebrew core·MCP Registry 모두 비어 있음. 크레이트 접미 이름(`-mcp`·`-cli`·`-core`)도 비어 있음 |
| 검색 구분 | "codereader mcp" 저장소 검색 0건 |
| 약점 | `search`·`grep`·`digest` 같은 검색·로그 압축 기능은 이름에 드러나지 않음 |

## 4. 벤치마크 자료 축소

회귀 게이트는 성능 저하를 막는 내부 검사라 유지합니다. 홍보용 수치와 정기 측정을 줄입니다.

| 대상 | 조치 |
|---|---|
| README 부제·첫 문단의 수치 4개 | 삭제. 한 줄 소개로 대체 |
| README "Performance & Token Savings" 절(66줄) | `docs/BENCHMARKS.md` 로 이동. README 에는 링크 1줄 |
| README 다른 도구 비교표(17행) | 삭제. "Other code-search tools: typically ✓" 처럼 측정 근거가 없는 칸이 있음 |
| README 막대그래프 이미지 `token_savings_chart.png` | README 에서 제거 |
| README 의 `GPT IMAGE PROMPT` 주석 4개 | 삭제 |
| README 설치 프롬프트 50줄 | `docs/AGENT_INSTALL.md` 로 이동. README 에는 설치 1줄과 등록 1줄 |
| GitHub 저장소 설명·Claude Code 플러그인 설명의 "84-88%"·"~85%" | 한 줄 소개 문장으로 교체 |
| `nightly-benchmark.yml` 매일 실행 | `schedule` 삭제, `workflow_dispatch`(수동 실행)만 유지 |
| `benchmark-results` 릴리스 태그 | 삭제 여부를 사용자가 결정 |
| `benchmarks/llm-bench/` (v0.6 비교용 골격) | 삭제 또는 보류. 로드맵 v0.8.0 의 LLM-in-loop bench 항목과 함께 결정 |
| 로드맵 v1.0.0 "benchmark dashboard" | 삭제 |
| `ci.yml` 의 `bench-regression`, `benchmarks/baseline.json`, `scripts/regression-gate.py` | 유지 |
| `CONTRIBUTING.md` "Performance claims" 규칙 | 유지. 수치를 쓸 때만 적용됨 |
| `CONTRIBUTING.md` 커밋 본문 수치 기재 요구 | 엔진 변경에만 적용하도록 문구 축소 |
| "1분 설치" 문구 | 설치부터 첫 응답까지 시간을 macOS·Linux 에서 측정한 뒤에만 사용. 첫 실행 모델 내려받기(약 60 MB) 시간 포함 |

## 5. 이름 변경 작업 목록

### 저장소·코드

| 순서 | 작업 | 대상 |
|---|---|---|
| 1 | 코드·문서의 이름 변경 PR 작성 | `Cargo.toml`(`repository`·`homepage`), 크레이트 이름 `pluck-*` → `open-codereader-*`, 바이너리 이름, README 2개, `docs/*`, `CONTRIBUTING.md`, `ROADMAP.md`, `CHANGELOG.md`, 이슈 템플릿 |
| 2 | 코드 안 이름 변경 | `pluck init` 이 쓰는 MCP 서버 이름, 도구 이름 `mcp__pluck__*`, `PLUCK_*` 이름 12개(환경 변수·설정 표식), 데이터 폴더 `~/.pluck`(`PLUCK_HOME`), `plugins/claude-code/` 템플릿·훅 |
| 3 | 기존 이름 호환 | 한 버전 동안 `pluck`·`pluckd` 명령 별칭, `PLUCK_*` 환경 변수와 `~/.pluck` 를 새 이름이 없을 때 읽기. `init` 이 기존 `pluck` MCP 등록을 찾아 새 이름으로 교체 |
| 4 | 릴리스 산출물 이름 변경 | `release.yml` 의 `pluck-<target>.tar.gz` |
| 5 | Claude Code 플러그인 | `.claude-plugin/marketplace.json`(`hunhee98-pluck`), `plugins/claude-code/.claude-plugin/plugin.json` 이름·설명 |

### 외부 공개 작업 (승인 후)

| 순서 | 작업 | 동작 |
|---|---|---|
| 6 | Homebrew | 새 탭 저장소 `hunhee98/homebrew-tap` 에 사전 빌드 Formula 추가. 기존 `homebrew-pluck` 탭에 `formula_renames.json` 또는 `tap_migrations.json` 으로 새 이름 연결 |
| 7 | GitHub 저장소 이름 변경 | Settings 에서 `hunhee98/pluck` → `hunhee98/open-codereader`. 설명·토픽·소셜 미리보기 이미지 교체 |
| 8 | crates.io | 크레이트 이름은 바꿀 수 없음. `open-codereader-core`·`-mcp`·`-cli` 를 새로 배포. 기존 `pluck-*` 는 마지막 버전 README·설명에 이전 안내를 넣어 배포하고 yank 하지 않음 |
| 9 | npm 래퍼 | 2절에서 npm 래퍼를 채택한 경우에만 배포 |
| 10 | 공식 MCP Registry 등록 | 새 이름으로 등록 |
| 11 | 기존 사용자 안내 | 릴리스 노트, `CHANGELOG.md`, 기존 크레이트 README, 고정 이슈 1개 |
| 12 | 이력서·외부 링크 | 이력서의 저장소 주소를 새 주소로 교체 |

### GitHub 이름 변경 후 리디렉션 동작

GitHub 공식 문서 "Renaming a repository" 기준입니다.

| 대상 | 동작 |
|---|---|
| 웹 주소 `github.com/hunhee98/pluck` | 새 주소로 리디렉션 |
| `git clone`·`git fetch`·`git push` 기존 주소 | 계속 동작. 로컬 원격 주소는 새 주소로 바꾸라고 권장 |
| 릴리스·이슈·PR 링크 | 새 주소로 리디렉션 |
| GitHub Pages 주소 | 리디렉션 안 됨. 이 저장소는 Pages 를 쓰지 않음 |
| 같은 계정에 `pluck` 이름 저장소를 새로 만들 때 | 기존 리디렉션이 끊김. 이름 변경 후 `hunhee98/pluck` 을 다시 만들지 않음 |
| `brew tap hunhee98/pluck` | 탭 저장소 `homebrew-pluck` 은 이번 변경과 별개 저장소라 그대로 동작. 작업 6에서 새 이름 연결 |
| `/plugin marketplace add hunhee98/pluck` | git clone 리디렉션으로 동작할 것으로 보지만 확인 필요 |

## 6. 사용자가 결정할 항목

| 순서 | 항목 | 추천 |
|---|---|---|
| 1 | 새 이름 | `open-codereader` |
| 2 | 명령·MCP 서버 이름도 바꿀지 | 바꿈. `pluck`·`pluckd` 는 한 버전 동안 별칭으로 유지 |
| 3 | 설치 한 줄 방식 | Homebrew 사전 빌드 Formula 먼저, npm 래퍼는 반응을 본 뒤 결정 |
| 4 | `benchmark-results` 태그·`benchmarks/llm-bench/` 처리 | 둘 다 삭제 |
| 5 | 커밋 작성자 정보 | 기존 커밋 작성자는 `Hunhee Lee <hunhee.dev@gmail.com>`. 같은 값을 쓸지 확인 필요 |
