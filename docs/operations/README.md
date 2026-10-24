# 成果物運用ドキュメント

## 対象読者

公開された CLI 成果物を導入し、設定し、利用し、稼働維持する利用者・運用者。

## 責務

成果物を導入し、設定し、利用し、稼働維持するための手順を扱う。

## 責務外

- リポジトリ自体の開発・保守。→ `docs/maintenance/`
- 版付け・出荷・公開。→ `docs/release/`

## 正本と導線

- 成果物の概要・できること・できないこと: `README.md`
- オフライン導入（対象別installer、事前取得manifest・archive）: `docs/operations/setup-release.md`
- 使い方（実行ガイド）:
  - BoundaryPolicy 入力: `skills/a3-rust-cruiser/references/run-boundary-policy-check.guide.md`
  - resolved-implementation-attribution 入力: `skills/a3-rust-cruiser/references/run-resolved-implementation-attribution-check.guide.md`
- BoundaryPolicy の作成: `skills/a3-rust-cruiser/references/create-boundary-policy.guide.md`
