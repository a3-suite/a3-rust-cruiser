---
name: local-install
description: a3-rust-cruiser の clone 済みソースを開発環境へローカルインストールするスクリプトを実行し、導入した CLI の最小プロジェクト検証まで確認するときに使う。GitHub Release 資産からの利用者向け導入には使わない。
---

# local-install / SKILL

<!-- skill-template: single-purpose -->

## 目的

- 現在の checkout から開発用 `a3-rust-cruiser` CLI を導入し、最小プロジェクトで利用可能な状態まで確認する。

## 原則

- 利用者向け配布は GitHub Release 資産を使用し、このスキルは代替しない。
- 実行入口は `scripts/install-local.sh` とする。
- install root の解決、Cargo install、smoke test の実装は同スクリプトを正本とする。

## 注意

- Rust toolchain、Cargo、依存 crate は導入せず、事前に利用可能であることを前提とする。
- アンインストール、公開、Git 操作は扱わない。
- 個別の Cargo コマンドへ分解するのは、スクリプト自体を診断するときだけにする。

## フロー

### ローカルインストール

- `bash {skill-root}/scripts/install-local.sh` を実行する。
- スクリプトが報告する実行ファイルパスと smoke test の `decision: conformant` を確認する。
- toolchain または Cargo が不足している場合は前提不足として報告する。
- Cargo install、実行ファイル確認、smoke test のどこで失敗したかを分けて報告し、別の install root や `--force` へ暗黙に切り替えない。
