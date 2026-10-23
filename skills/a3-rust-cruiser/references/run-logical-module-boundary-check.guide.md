# 論理モジュール境界検証の共通判定ガイド

## 目的

- 入力経路別のガイドで実行した論理モジュール境界検証の結果を判定する。
- 適合、違反、解析不能、実行エラーを混同せずに判定する。

## 前提条件

- YAML BoundaryPolicyまたはresolved-implementation-attributionの入力経路別ガイドに従い、入力と解析コンテキストを確定済みであること。
- `a3-rust-cruiser` コマンドが利用できること。利用できない場合はセットアップを自動実行せず、不足を報告する。

## 判定基準

- 終了コード `0` / `decision: conformant`: 指定した解析コンテキストでは適合。
- 終了コード `1` / `decision: non-conformant`: `violations` を根拠に境界違反または循環依存を報告する。
- 終了コード `2` / `decision: indeterminate`: `analysis_issues` と影響範囲を報告し、適合とは判定しない。
- 終了コード `3`: 引数、入力、解析、または出力処理のエラーとして標準エラーを報告する。
- 終了コードと `decision` が対応しない場合は、結果を採用せず契約不整合として報告する。

## 大規模レポートの読み方

JSONレポートが大きく、全文を出力として扱うと切り詰められる場合に適用する。全文を確認できる場合は従来どおり標準出力を読む。

1. 同じ実行の標準出力（`--format json`）を、作業中に確定した一時パスへ一度だけ保存し、その終了コードを同時に記録する。保存したJSONと終了コードを1組の結果として扱う。
2. 保存したJSONから、最初に `decision`、`violations` の件数、`analysis_issues` の `code` 別件数と `policy_impact.outcome` 別件数を集計する。
3. 詳細は、判断に必要な rule ID、issue code、`affected_scope` と代表例だけを同じJSONから取り出す。
4. 実行ファイル、入力、解析コンテキストが変わっていない限り、別の集計のために check を再実行しない。
5. 一部だけを取り出した結果を完全なレポートとして扱わない。判定は「判定基準」に従い、件数だけで `analysis_issues` を無視しない。
6. 不要になった一時ファイルは、手順1で確定した正確なパスだけを削除する。

## 問題・エラー時の切り分け

最初に、実行コマンド、終了コード、`decision`、標準エラー、JSONの `analysis_issues`、package・target・target triple・feature・cfg・include・excludeを記録する。問題の分類を推測だけで確定せず、次の順で調査する。

| 観測結果 | 先に調べる場所 | 修正候補 |
| --- | --- | --- |
| コマンド未検出、version不一致、platform非対応、checksum・PATH・install directoryエラー | `references/setup-release.guide.md`、実行ファイルのversion・実体・PATH | セットアップまたは実行環境。policy入力は変更しない |
| 終了コード `3`、policy入力の欠落・形式不正・引数不正 | 指定した入力パス、YAML boundary policy、resolved-implementation-attribution、package/target指定 | 入力定義または実行指定。正しい入力をツールが拒否する場合だけツールの入力検証を修正候補にする |
| `non-conformant` と `violations` | 違反のrule ID、依存元・依存先、証拠位置、policyのfrom/to、解決済み帰属 | 依存方向が意図に反するなら対象ソースまたはpolicyを修正し、ruleやowner出力が意図と違うなら各入力のownerを調査する |
| `indeterminate` と `analysis.unsupported-analysis-context` | target triple、Cargo feature、rustc cfg、target dependency、`analysis_issues`のaffected scope | 実行コンテキストが不足していれば呼び出し側の指定を修正し、指定済みの有効条件をツールが評価できないならツールの解析境界を修正候補にする |
| `indeterminate` と unsupported macro/name/syntax issue | issue code、reason、affected scope、evidence、`policy_impact`、対象ソースの構文 | 依存を生じないことが契約上証明できる構文ならツールの分類不足を修正候補にし、依存先を変え得る未対応構文は適合へ読み替えず対象ソースまたは解析境界を見直す |
| `conformant` だが期待と異なる | package/target、include/exclude、policy scope、`normalized_sdd_coverage`、JSONの modules/dependencies | 対象範囲や定義が誤っていれば入力定義を修正し、同一の正しい入力を最小fixtureでも誤判定するならツールを修正候補にする |
| 終了コードと `decision` の不一致、JSON schema・orderingの破損 | CLI report、終了コード契約、同一入力の再実行結果 | ツールまたは公開契約の不整合。policy入力の変更で隠さない |

各問題は、次の形式で報告する。

- 事象: コマンド、終了コード、`decision`、主要なissue/violation。
- 原因分類: `tool`、`input-definition`、`target-source`、`environment`、`undetermined` のいずれか。
- 調査先: 実行ファイル、解析コンテキスト、policy、resolved-implementation-attribution、対象ソースの具体的なパス。
- 修正対象: ツール、入力定義、対象ソース、実行指定のいずれか。まだ確定できない場合は無理に修正しない。
- 再検証: 同じ解析コンテキストでの再実行、または原因を分離する最小fixtureと回帰テスト。

`violations` が空であること、issue件数が減ったこと、`--exclude` で問題が消えたことだけを修正完了の根拠にしない。

## 出力/完了条件

- 実行した解析コンテキストを package、target、target triple、analysis profile とともに示す。
- 使用したpolicy入力をYAMLまたはresolved-implementation-attributionとして示す。resolved-implementation-attribution入力ではowner生成済み一時出力のパスとcoverage結果を示す。
- `non-conformant` では rule ID、依存元、依存先、根拠位置を示す。
- `indeterminate` では analysis issue code、理由、影響範囲を示す。
- 問題やエラーがある場合は、原因分類、調査先、修正対象、再検証方法を示す。
- 対象としたすべての解析コンテキストが `conformant` の場合だけ、検証完了とする。

## 注意

- `source-union` profile は、対象プロジェクトが非コンパイル経路を含むソース集合の解析を明示的に求める場合だけ使用する。
- `--policy`と`--resolved-implementation-attribution`は相互排他とする。
- `--exclude` によって消えた依存は検証対象外であり、依存が存在しないことの証拠にはならない。
- 空の `violations` だけを見て適合と判断せず、必ず `decision` と `analysis_issues` を確認する。
