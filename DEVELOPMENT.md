# 開発ガイド

## 対象

このリポジトリを開発・保守する人とコントリビューター向けの参照入口です。

## 読む順序

1. [保守ドキュメント](docs/maintenance/README.md)で開発・検証の正本を確認する。
2. 変更する領域に応じて、[CI運用](.ci/README.md)、[リリース文書](docs/release/README.md)、または下記のSSOTを参照する。

## SSOT

| 領域 | 正本 |
| --- | --- |
| 要求・仕様・設計 | `sdd/dsl/` の current artifact |
| Rust package、依存、toolchain | `Cargo.toml`、`Cargo.lock`、`rust-toolchain.toml` |
| 品質ゲートの実行定義 | `.github/workflows/quality-gate.yml` |
| lint設定 | `a3-lint.yaml` |
| 契約対象回帰の範囲と実行定義 | `tests/contract-subject-regression.yml` |
| CIの採用preset、所有境界、検証導線 | `.ci/README.md` |

## 最低限の運用ルール

- このガイドと正本の記載が競合した場合は、正本を優先する。
- 詳細な手順とコマンドは、各正本またはリンク先の運用文書を参照する。

## 関連する入口

- [保守ドキュメント](docs/maintenance/README.md)
- [CI運用](.ci/README.md)
- [リリース文書](docs/release/README.md)
