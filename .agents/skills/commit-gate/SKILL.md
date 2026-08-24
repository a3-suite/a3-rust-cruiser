---
name: commit-gate
description: a3-rust-cruiser でコミット前に staged snapshot の変更を検証するスクリプトを実行し、local-install 契約に基づいてコミット可否を判定するときに使う。
---

# commit-gate / SKILL

## 目的

- staged snapshot から CLI をローカルインストールし、導入後も利用できることを確認してコミット可否を判定する。

## 原則

- 検証対象は worktree ではなく staged snapshot とする。
- インストールと smoke test の内容は local-install スキルへ委譲し、このスキルでは再定義しない。
- local-install スキルが解決した通常の Cargo install root へ導入し、ゲート終了後も CLI を残す。

## 関連スキル

- 必須: git スキル - 基本コミットゲートとコミット操作を所有するため
- 必須: local-install スキル - インストールと smoke test の正本を所有するため

## 注意

- staged 変更が無い場合は停止する。
- staged snapshot を分離できない場合や一時領域を安全に後始末できない場合は、コミット可能と判定しない。
- local-install スキルが定める外部書き込みの承認条件を満たしてから実行する。
- Cargo が利用する既存の registry cache は、Cargo 自身の管理境界とする。

## トリガー＆アクション

### a3-rust-cruiser をコミットする前

- git スキルの基本コミットゲートを適用する。
- `bash {skill-root}/scripts/run-commit-gate.sh` を実行する。
- 終了コード `0` の場合だけコミット可能とする。`1` は検証不合格、`2` は判定不能として停止する。
