# 保守ドキュメント

## 対象読者

このリポジトリを変更・検証・維持する保守者とコントリビューター。

## 責務

プロジェクト自体を変更し、検証し、維持するための手順と規則を扱う。

## 責務外

- 成果物（CLI）の利用者向け導入・設定・利用。→ `docs/operations/`
- 版付け・出荷・公開。→ `docs/release/`

## 正本と導線

- 開発前提と固定 toolchain: `Cargo.toml`、`rust-toolchain.toml`
- 品質ゲート（format / lint / test / security / structure）: `.github/workflows/quality-gate.yml`（標準 bundle `rust-cargo-quality` を選択）、配布同一性 `.ci/ci-assets.lock.json`
- lint 設定: `a3-lint.yaml`（実行前に、`rust` スキルを含むスキル置き場を `A3_SKILLS_ROOT` に設定する）
- SDD 検証: `sdd/`（`a3-sdd sdd check`。実行前に、SDD owner skill の置き場を `A3_SDD_OWNER_SKILL_DISCOVERY_ROOT` に設定する。参照箇所は `sdd/dsl/core/sdd-owner-skill-discovery.sdd.yml`）
- 契約対象回帰の実行定義: `tests/contract-subject-regression.yml`
- コミット前ゲートとローカル導入: `.agents/skills/commit-gate/SKILL.md`、`.agents/skills/local-install/SKILL.md`
