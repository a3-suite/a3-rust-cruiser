# CI 運用

## 固定項目（必須）

### 目的と対象

Rust CLI（a3-rust-cruiser）の品質検証と、GitHub Release assets（3 platform の CLI archive、target-specific installer、manifest、checksum、組立証跡）の build・検証・公開を対象とする。

### 採用 preset / flow

canonical preset `quality-gate`、`release-request`（tag variant）、`release-publication`（request / caller + provider reusable publication）を採用する。品質と Release 公開の実処理は provider reusable workflow（`a3-suite/a3-ci-github`、固定 full commit SHA）が所有し、consumer は薄い caller だけを配置する。package publication は採用しない。

### project 固有差分

- 差分種別: project-owned contract-check workflow
- 標準で成立しない理由: provider の標準 Rust quality bundle は project の CI / installer 構成契約を検査せず、canonical workflow への独自 job 追加は配布契約に反するため。
- owner: release maintainers
- 正本・検証導線: `Cargo.toml`、`rust-toolchain.toml`、`.ci/platform-manifest.yml`、`.ci/ci-assets.lock.json`、`.ci/provider-action-pins.yml`、`installer/assembly.json`、`installer/manifests/`、`tests/ci/`
- 検証証跡: `actionlint`、CI 用 `a3-lint`、provider preflight、`tests/ci/run-ci-contract-tests.sh`、`a3-sdd sdd check` をローカル実行する。Hosted runner と required check・Repository 権限はリリース前に GitHub 側で確認し、Release asset の remote readback は公開後に確認する。
- 更新・撤去条件: manifest、Rust toolchain、runner、Action SHA、provider revision、installer 配布経路を変更するときに宣言・lock・契約検証を同時に更新する。製品 version を更新するときは各 manifest の release path、`installer/assembly.json` の共有 wrapper URL、`README.md` の製品 version・導入手順、`docs/operations/setup-release.md` の導入手順を揃える。Release asset 経路を撤去するときは利用者向け導線も更新する。

installer は固定 provider revision の `standard-installer` を使い、caller は `installer/assembly.json` を指定する。Unix は共有 `install.sh` がOS・CPUに対応する対象別 installer を取得・検証して実行し、Windows は対象別 `install.ps1` を使う。製品側では assembly 宣言と Linux x86_64・macOS arm64・Windows x86_64 の manifest、対象別 installer asset を管理する。共通 runtime、builder、installer tests、adapter は provider が所有する。

Release の authority / source-gate / build / assemble / publish は provider reusable workflow と固定 SHA Action が所有し、project 側に対応する entrypoint を置かない。trigger は quality-gate が pull_request と push(`main`)、project-contracts が pull_request と push(`main`)、release-request が tag `v*`、release-publication が manual dispatch（`release-publication-request`）と `workflow_run`（caller）である。project-contracts は `tests/ci/run-ci-contract-tests.sh` を実行し、provider canonical workflow から独立した project-owned check を公開する。
