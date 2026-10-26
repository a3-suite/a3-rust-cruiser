---
name: commit-gate
description: a3-rust-cruiser でコミット前に staged snapshot の変更を検証するスクリプトを実行し、local-install 契約に基づいてコミット可否を判定するときに使う。
---

# commit-gate / SKILL

## 目的

- staged snapshotでCLIへ影響する変更を判定し、必要な場合だけローカルインストールしたうえで利用可能性を確認する。

## 原則

- 検証対象は worktree ではなく staged snapshot とする。
- インストールとsmoke testの内容はlocal-installスキルへ委譲し、このスキルでは再定義しない。
- 既知の非バイナリ変更だけなら既存CLIを検証し、実行ファイルへ影響する変更または未知のパスを含む場合は通常のCargo install rootへ導入する。
- 影響判定は安全側とし、判定できない変更をインストール不要として扱わない。

## ローカルゲートの結果

- 期待するゲートは `a3-rust-cruiser.local-install` の1件。
- 適用条件: 本リポジトリの staged snapshot をコミットする場合。非バイナリ変更でも既存 CLI の検証を行うため、非適用にはしない。
- このスキルの実行主体は、スクリプトの実行後に次の対応で1件の `local_gate_result` を git スキルへ返す。
  - `gate_id`: `a3-rust-cruiser.local-install`
  - `status`: 終了コード `0` は `PASS`、`1` は `STOP`、`2` またはその他の異常終了は `判定不能`。
  - `reason`: 実際に通った導入または既存 CLI 検証の経路と、成功・停止の理由。
  - `evidence`: 検証した staged tree ID、スクリプトの実行結果・終了コード、実行ファイルと smoke test の結果。実行前に失敗した項目は未確認として示す。
- スクリプトの終了コードと出力を根拠に返却し、失敗や後始末の不備を成功へ読み替えない。

## 責務境界

- 所有: staged snapshot に対する CLI 導入要否の判定と、local-install の実行結果の返却
- 非対象: 基本コミットゲートとコミット操作（git スキル）、導入・smoke test の定義（local-install スキル）

## 関連スキル

- 必須: git, local-install

## 注意

- staged 変更が無い場合は停止する。
- staged snapshot を分離できない場合や一時領域を安全に後始末できない場合は、コミット可能と判定しない。
- local-install スキルが定める install 条件を満たしてから実行する。
- Cargo が利用する既存の registry cache は、Cargo 自身の管理境界とする。

## トリガー＆アクション

### a3-rust-cruiser をコミットする前

- git スキルの基本コミットゲートを適用する。
- `{skill-root}/scripts/run-commit-gate.sh` を実行する。
- 終了コード `0` の場合だけコミット可能とする。`1` は検証不合格、`2` は判定不能として停止する。
