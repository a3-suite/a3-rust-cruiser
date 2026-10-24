---
name: a3-rust-cruiser
description: 非SDDのRustプロジェクトで決定した境界ルールからYAML boundary policyを作成・修正するとき、またはpolicyやowner生成済みの解決済み実装帰属で論理モジュール依存、境界違反、循環依存を検証するときに使う。
---

# a3-rust-cruiser / SKILL

## 目的

- Rust プロジェクトの論理モジュール境界を、プロジェクトが所有する YAML boundary policyまたはowner生成済みの解決済み実装帰属に対して検証する。
- 検証結果を `conformant`、`non-conformant`、`indeterminate` に区別し、根拠とともに報告する。

## 原則

- 解析対象は、1回の実行につき1つの Cargo package と compilation target に限定する。
- 境界入力、package、target、target triple は推測せず、対象プロジェクトとツールチェーンから確認する。
- YAML boundary policyはSDDを正本として運用しないプロジェクトの入力として扱う。SDDプロジェクトでは作成・使用しない。
- 解決済み実装帰属は、`a3-sdd`の正規導線で生成されたowner出力を変換せずに使用する。
- `indeterminate` と実行エラーを適合として扱わない。
- 除外指定を、違反を隠して適合させる目的で追加しない。

## 責務境界

- 所有: Rust プロジェクトの論理モジュール境界の検証と結果の判定、非SDDプロジェクトの YAML BoundaryPolicy の作成・修正
- 非対象: crate 間や workspace package 間の依存検証
- 非対象: SDD DSLの意味、current artifactの解決、解決済み実装帰属の生成（`a3-sdd`、`sdd-designs`）
- 非対象: プロジェクトのアーキテクチャ方針の独自決定、a3-rust-cruiser 自体の開発と導入

## トリガー＆アクション

### YAML BoundaryPolicyで論理モジュール境界を検証したい

- 対象プロジェクトが所有する YAML BoundaryPolicy と解析コンテキストを確定する。
- 参照: references/run-boundary-policy-check.guide.md

### YAML BoundaryPolicyを作成・修正したい

- 非SDDプロジェクトで、根拠のある境界ルールをYAMLへ記述し、検証へ進む。
- 参照: references/create-boundary-policy.guide.md

### resolved-implementation-attributionで論理モジュール境界を検証したい

- SDDプロジェクトではこの経路を選ぶ。owner出力が不足している場合は停止し、YAML BoundaryPolicyへ切り替えない。
- 参照: references/run-resolved-implementation-attribution-check.guide.md

### 境界検証が完了したか判断したい

- `decision`、`violations`、`analysis_issues`、解析コンテキストを確認する。
- `non-conformant`、`indeterminate`、実行エラーをそれぞれ区別して未解決事項を報告する。
- 問題やエラーがある場合は、ツール、入力定義、対象ソース、実行環境のどれが原因候補かを切り分け、調査先、修正対象、再検証方法を示す。
- 参照: references/run-logical-module-boundary-check.guide.md
