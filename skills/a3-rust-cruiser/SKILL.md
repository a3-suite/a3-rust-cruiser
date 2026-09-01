---
name: a3-rust-cruiser
description: GitHub Releaseの固定版a3-rust-cruiser CLIをセットアップし、YAML boundary policyまたはowner生成済みの解決済み実装帰属を使ってRustプロジェクトの論理モジュール依存、境界違反、循環依存、解析不能を検証するときに使う。
---

# a3-rust-cruiser / SKILL

## 目的

- Rust プロジェクトの論理モジュール境界を、プロジェクトが所有する YAML boundary policyまたはowner生成済みの解決済み実装帰属に対して検証する。
- GitHub Release の固定版CLIをchecksum検証後に導入し、導入versionとcommand pathを確認する。
- 検証結果を `conformant`、`non-conformant`、`indeterminate` に区別し、根拠とともに報告する。

## 原則

- 解析対象は、1回の実行につき1つの Cargo package と compilation target に限定する。
- 境界入力、package、target、target triple は推測せず、対象プロジェクトとツールチェーンから確認する。
- YAML boundary policyはSDDに依存しない標準入力として扱う。
- 解決済み実装帰属はowner生成済みの一時出力を変換せずに使用する。current artifact familyの定義・選択規則と導出意味は`sdd-designs`、current artifact instanceの解決・family validation・runtime operationの実行は`a3-lint`へ委譲し、a3-rust-cruiserはraw owner DSLの解釈や帰属の再計算を行わない。package、target、target triple、feature、cfg、analysis profile、include、excludeはCLIの解析コンテキストとして別に確定する。
- セットアップでは固定versionだけを使用し、`latest`やbranchから取得しない。
- `indeterminate` と実行エラーを適合として扱わない。
- 除外指定を、違反を隠して適合させる目的で追加しない。

## 注意

- 非対象: crate 間や workspace package 間の依存検証。
- 非対象: boundary policy の新規設計、a3-rust-cruiser 自体の開発。
- 対応platformはLinux x86_64とmacOS arm64に限定する。
- macro expansion など解析できない構造がある場合、違反が0件でも検証完了とは扱わない。

## トリガー＆アクション

### a3-rust-cruiserをセットアップしたい

- 固定version、対応platform、既存CLI、install directory、PATHを事前確認する。
- 外部取得とinstall directoryへの書き込みについて利用者の承認後、checksum検証を伴うセットアップを実行する。
- 参照: references/setup-release.guide.md

### YAML BoundaryPolicyで論理モジュール境界を検証したい

- 対象プロジェクトが所有する YAML BoundaryPolicy と解析コンテキストを確定する。
- 参照: references/run-boundary-policy-check.guide.md

### resolved-implementation-attributionで論理モジュール境界を検証したい

- 対象プロジェクトにcurrent SDD design familyがある場合だけこの経路を選び、ない場合はYAML BoundaryPolicyのガイドへ切り替える。
- 生成が必要なら、ガイドに従って`sdd-designs`が定義する`projection materialize`を`a3-lint`へ委譲し、実行結果の`resolved_output_path`を受け取る。
- owner生成済みの resolved-implementation-attribution と解析コンテキストを確定する。
- 入力を変換せず a3-rust-cruiser を実行し、coverageを含むレポートを判定する。
- 参照: references/run-resolved-implementation-attribution-check.guide.md

### 境界検証が完了したか判断したい

- `decision`、`violations`、`analysis_issues`、解析コンテキストを確認する。
- `non-conformant`、`indeterminate`、実行エラーをそれぞれ区別して未解決事項を報告する。
- 問題やエラーがある場合は、ツール、入力定義、対象ソース、実行環境のどれが原因候補かを切り分け、調査先、修正対象、再検証方法を示す。
- 参照: references/run-logical-module-boundary-check.guide.md
