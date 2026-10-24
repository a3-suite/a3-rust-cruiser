# リリースドキュメント

## 対象読者

プロジェクト成果物を版付けし、出荷・公開し、公開結果を確認するリリース担当の保守者。

## 責務

プロジェクト成果物を版付けし、出荷・公開し、公開結果を確認するための手順を扱う。

## 責務外

- リポジトリの日常的な保守。→ `docs/maintenance/`
- 公開後の成果物の継続運用。→ `docs/operations/`

## 正本と導線

- CI・リリース運用の正本: `.ci/README.md`
- 採用プリセットと公開経路:
  - `release-request`（tag variant）: `.github/workflows/release-request-tag.yml`
  - `release-publication`（request / caller、実処理は provider reusable `a3-suite/a3-ci-github/.github/workflows/ci-release-publication.yml`）: `.github/workflows/release-publication-request.yml`、`.github/workflows/release-publication-caller.yml`
- 3 platform の CLI archive、target-specific installer、Unix共有wrapper: `.ci/platform-manifest.yml`、`.ci/installer/assembly.json`、`.ci/installer/manifests/`
- 利用者向け導入手順: `docs/operations/setup-release.md`

Release authority、build、assembly、publish、remote readback は provider reusable workflow と固定 SHA Action が所有する。consumer 側の CI 設定と検証範囲は `.ci/README.md` を参照する。
