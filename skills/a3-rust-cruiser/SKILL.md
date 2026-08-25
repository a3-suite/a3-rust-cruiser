---
name: a3-rust-cruiser
description: GitHub Releaseの固定版a3-rust-cruiser CLIをセットアップし、Rustプロジェクトの論理モジュール依存、境界違反、循環依存、解析不能を検証するときに使う。
---

# a3-rust-cruiser / SKILL

## 目的

- Rust プロジェクトの論理モジュール境界を、プロジェクトが所有する YAML boundary policy または SDD 設計入力に対して検証する。
- GitHub Release の固定版CLIをchecksum検証後に導入し、導入versionとcommand pathを確認する。
- 検証結果を `conformant`、`non-conformant`、`indeterminate` に区別し、根拠とともに報告する。

## 原則

- 解析対象は、1回の実行につき1つの Cargo package と compilation target に限定する。
- policy入力、package、target、target triple は推測せず、対象プロジェクトとツールチェーンから確認する。
- YAML boundary policyはSDDに依存しない標準入力として扱い、対象プロジェクトがSDD設計を所有しない場合にSDD入力を要求しない。
- セットアップでは固定versionだけを使用し、`latest`やbranchから取得しない。
- `indeterminate` と実行エラーを適合として扱わない。
- SDDのmany-to-one projectionまたは親子関係で重なるRust module scopeは入力不正とせず、component帰属が曖昧な解析範囲を`analysis_issues`で確認する。
- SDDのproject全体projectionにある正規source nodeのうち、Rust module解析対象外と確定できるnodeはboundary policy導出対象から除外する。
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

### Rust の論理モジュール境界を検証したい

- 対象プロジェクトが所有する YAML boundary policy、または logical-structure、projection、directory-structure の組と解析コンテキストを確定する。
- a3-rust-cruiser を実行し、終了コードとレポートを合わせて判定する。
- 参照: references/run-logical-module-boundary-check.guide.md

### 境界検証が完了したか判断したい

- `decision`、`violations`、`analysis_issues`、解析コンテキストを確認する。
- `non-conformant`、`indeterminate`、実行エラーをそれぞれ区別して未解決事項を報告する。
- 参照: references/run-logical-module-boundary-check.guide.md
