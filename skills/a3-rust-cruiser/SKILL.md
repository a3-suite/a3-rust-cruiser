---
name: a3-rust-cruiser
description: 非SDDのRustプロジェクトで決定した境界ルールからYAML boundary policyを作成・修正するとき、GitHub Releaseの固定版a3-rust-cruiser CLIを標準installer bundleからセットアップするとき、またはpolicyやowner生成済みの解決済み実装帰属で論理モジュール依存、境界違反、循環依存を検証するときに使う。
---

# a3-rust-cruiser / SKILL

## 目的

- Rust プロジェクトの論理モジュール境界を、プロジェクトが所有する YAML boundary policyまたはowner生成済みの解決済み実装帰属に対して検証する。
- GitHub Release の標準 installer bundle と固定版CLI archiveをchecksum検証後に導入し、導入versionと実行pathを確認する。
- 検証結果を `conformant`、`non-conformant`、`indeterminate` に区別し、根拠とともに報告する。

## 原則

- 解析対象は、1回の実行につき1つの Cargo package と compilation target に限定する。
- 境界入力、package、target、target triple は推測せず、対象プロジェクトとツールチェーンから確認する。
- YAML boundary policyはSDDを正本として運用しないプロジェクトの入力として扱う。SDDプロジェクトでは作成・使用しない。
- 解決済み実装帰属は、`a3-sdd`の正規導線で生成されたowner出力を変換せずに使用する。SDD DSLの意味、current artifactの解決、出力方法は`a3-sdd`および`sdd-designs`へ委譲し、a3-rust-cruiserはraw owner DSLの解釈や帰属の再計算を行わない。package、target、target triple、feature、cfg、analysis profile、include、excludeはCLIの解析コンテキストとして別に確定する。
- セットアップでは固定versionだけを使用し、`latest`やbranchから取得しない。
- `indeterminate` と実行エラーを適合として扱わない。
- 除外指定を、違反を隠して適合させる目的で追加しない。

## 注意

- 非対象: crate 間や workspace package 間の依存検証。
- 非対象: プロジェクトのアーキテクチャ方針の独自決定、a3-rust-cruiser 自体の開発。
- 対応platformはLinux x86_64、macOS arm64、Windows x86_64。
- macro expansion など解析できない構造がある場合、違反が0件でも検証完了とは扱わない。

## トリガー＆アクション

### a3-rust-cruiserをセットアップしたい

- 固定versionと対応platformを確認し、標準 installer bundle と固定 manifest / CLI archive を checksum 検証してからセットアップする。
- 既定は per-user CLI profile とし、PATH は自動変更しない。取得済みの manifest と CLI archive を使う場合は offline mode を選ぶ。
- 参照: references/setup-release.guide.md

### YAML BoundaryPolicyで論理モジュール境界を検証したい

- 対象プロジェクトが所有する YAML BoundaryPolicy と解析コンテキストを確定する。
- 参照: references/run-boundary-policy-check.guide.md

### YAML BoundaryPolicyを作成・修正したい

- 非SDDプロジェクトで、根拠のある境界ルールをYAMLへ記述し、検証へ進む。
- 参照: references/create-boundary-policy.guide.md

### resolved-implementation-attributionで論理モジュール境界を検証したい

- SDDプロジェクトではこの経路を選ぶ。current SDD design familyやowner出力が不足している場合は停止し、YAML BoundaryPolicyへ切り替えない。
- `a3-sdd`の正規導線で生成されたresolved-implementation-attributionを受け取る。
- owner生成済みの resolved-implementation-attribution と解析コンテキストを確定する。
- 入力を変換せず a3-rust-cruiser を実行し、coverageを含むレポートを判定する。
- 参照: references/run-resolved-implementation-attribution-check.guide.md

### 境界検証が完了したか判断したい

- `decision`、`violations`、`analysis_issues`、解析コンテキストを確認する。
- `non-conformant`、`indeterminate`、実行エラーをそれぞれ区別して未解決事項を報告する。
- 問題やエラーがある場合は、ツール、入力定義、対象ソース、実行環境のどれが原因候補かを切り分け、調査先、修正対象、再検証方法を示す。
- 参照: references/run-logical-module-boundary-check.guide.md
