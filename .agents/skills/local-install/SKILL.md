---
name: local-install
description: a3-rust-cruiser の clone 済みソースを開発環境へローカルインストールするスクリプトを実行し、導入した CLI の最小プロジェクト検証まで確認するときに使う。GitHub Release 資産からの利用者向け導入には使わない。
---

# local-install / SKILL

<!-- skill-template: single-purpose -->

## 目的

- 現在の checkout から開発用 `a3-rust-cruiser` CLI を導入し、導入後もコマンド名で利用できる状態まで確認する。

## 原則

- 利用者向け配布は GitHub Release 資産を使用し、このスキルは代替しない。
- 実行入口は `scripts/install-local.sh` とする。
- install root の解決、Cargo install、smoke test の実装は同スクリプトを正本とする。
- 一時 install root で検証に合格してから、通常の install root へ導入する。
- 通常のローカルインストールでは、同じバージョンが導入済みでも現在の checkout で置き換える。
- `--verify-installed` では通常のinstall rootにある既存CLIの実体、PATH、smoke testだけを確認し、置き換えない。

## 注意

- Rust toolchain、Cargo、依存 crate は導入せず、事前に利用可能であることを前提とする。
- アンインストール、公開、Git 操作は扱わない。
- 解決した Cargo install root へ書き込み、既存の `a3-rust-cruiser` を置き換える可能性がある。
- 通常の install root への書き込みと既存 CLI の置き換えは、このスキルを明示的に実行した依頼の範囲として扱い、都度の追加承認を求めずに実行する。
- shell profile は変更しない。install root の `bin` が `PATH` に無い場合は成功としない。

## フロー

### ローカルインストール

- `bash {skill-root}/scripts/install-local.sh` を実行する。
- スクリプトが報告する実行ファイルパス、コマンド名による解決、smoke test の `decision: conformant` を確認する。
- toolchain または Cargo が不足している場合は前提不足として報告する。
- Cargo install、コマンド解決、実行ファイル確認、smoke test のどこで失敗したかを分けて報告し、別の install root へ暗黙に切り替えない。

### 既存インストールの検証

- 実行ファイルへ影響する入力が変わらない場合は、`bash {skill-root}/scripts/install-local.sh --verify-installed`を実行する。
- 通常のinstall rootにCLIが存在し、コマンド名で同じ実体へ解決され、smoke testが`conformant`になることを確認する。
- CLIが未導入または別の実体でshadowされている場合は失敗し、別のinstall rootや再インストールへ暗黙に切り替えない。
