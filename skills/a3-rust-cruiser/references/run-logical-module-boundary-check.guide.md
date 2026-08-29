# 論理モジュール境界検証ガイド

## 目的

- インストール済みの a3-rust-cruiser を使い、1つの Cargo package と compilation target の論理モジュール境界を検証する。
- 適合、違反、解析不能、実行エラーを混同せずに判定する。

## 前提条件

- `a3-rust-cruiser` コマンドが利用できること。利用できない場合はインストールを自動実行せず、その不足を報告する。
- 対象プロジェクトが所有する YAML boundary policy、または SDD の logical-structure、projection、directory-structure の3パスを確認できること。YAML入力にSDDは不要とし、どちらも確認できない場合は入力を推測または新規作成せず、入力不足として停止する。

## 検証フロー

1. 対象プロジェクトから project root、policy入力、package、compilation target を確認する。policy入力はYAML 1ファイル、またはSDD 3ファイルのどちらか一方に固定する。
2. 対象ツールチェーンから target triple を確認する。
3. SDDを使用しない標準的な検証では、既定の `compilation` profileでYAML boundary policyを直接指定する。機械判定が必要な場合はJSONを選ぶ。

   ```shell
   a3-rust-cruiser check <project-root> --policy <boundary-policy-path> --package <package> --target <target-kind>:<target-name> --target-triple <target-triple> --format json
   ```

   SDD設計を正本とするプロジェクトでは、派生YAMLを保存せず次の形式で直接検証する。

   ```shell
   a3-rust-cruiser check <project-root> --logical-structure <logical-structure-path> --projection <projection-path> --directory-structure <directory-structure-path> --package <package> --target <target-kind>:<target-name> --target-triple <target-triple> --format json
   ```

   project全体のprojectionにRust module解析対象外の正規source nodeが含まれる場合、そのnodeが入力不正にならずboundary policy導出対象から除外されることを確認する。
   projectionで1つのlogical componentが複数directory nodeへ対応する場合は、その全Rust module scopeを同じcomponentとして検証する。複数のlogical componentが同じdirectory nodeへ対応する場合、または異なるcomponentのRust module scopeが親子関係で重なる場合は有効入力として実行し、`analysis.ambiguous-component-attribution`の対象module・component集合と`policy_impact`を確認する。検証済み宣言DAGの部分graphとして全帰属候補を評価し、評価が同じ場合は確定判断する。候補間差分がある場合は`observation.component_dependency_candidates`でsource/target component候補ごとの`allowed` / `forbidden`を確認する。これらはBoundaryPolicy ruleではないため、`policy_impact.rule_ids`が空でも同fieldを根拠として追跡する。候補間差分または候補の完全列挙不能がある場合は終了コード`2` / `decision: indeterminate`になる。

4. 対象プロジェクトが feature、cfg、解析範囲を明示している場合だけ、対応する `--feature`、`--cfg`、`--include`、`--exclude` を追加する。
5. 複数の package または target が対象なら、解析コンテキストごとに独立して実行する。

## 判定基準

- 終了コード `0` / `decision: conformant`: 指定した解析コンテキストでは適合。
- 終了コード `1` / `decision: non-conformant`: `violations` を根拠に境界違反または循環依存を報告する。
- 終了コード `2` / `decision: indeterminate`: `analysis_issues` と影響範囲を報告し、適合とは判定しない。
- 終了コード `3`: 引数、入力、解析、または出力処理のエラーとして標準エラーを報告する。
- 終了コードと `decision` が対応しない場合は、結果を採用せず契約不整合として報告する。

## 問題・エラー時の切り分け

最初に、実行コマンド、終了コード、`decision`、標準エラー、JSONの `analysis_issues`、package・target・target triple・feature・cfg・include・excludeを記録する。問題の分類を推測だけで確定せず、次の順で調査する。

| 観測結果 | 先に調べる場所 | 修正候補 |
| --- | --- | --- |
| コマンド未検出、version不一致、platform非対応、checksum・PATH・install directoryエラー | `references/setup-release.guide.md`、実行ファイルのversion・実体・PATH | セットアップまたは実行環境。boundary policyやSDDは変更しない |
| 終了コード `3`、policy/SDDの欠落・形式不正・引数不正 | 指定した入力パス、YAML boundary policy、SDDの3正本、package/target指定 | 入力定義または実行指定。正しい入力をツールが拒否する場合だけツールの入力検証を修正候補にする |
| `non-conformant` と `violations` | 違反のrule ID、依存元・依存先、証拠位置、policyのfrom/to、SDD projection | 依存方向が意図に反するなら対象ソースまたは設計を修正し、ruleや帰属が意図と違うならpolicy/SDDを修正する |
| `indeterminate` と `analysis.unsupported-analysis-context` | target triple、Cargo feature、rustc cfg、target dependency、`analysis_issues`のaffected scope | 実行コンテキストが不足していれば呼び出し側の指定を修正し、指定済みの有効条件をツールが評価できないならツールの解析境界を修正候補にする |
| `indeterminate` と unsupported macro/name/syntax issue | issue code、reason、affected scope、evidence、`policy_impact`、対象ソースの構文 | 依存を生じないことが契約上証明できる構文ならツールの分類不足を修正候補にし、依存先を変え得る未対応構文は適合へ読み替えず対象ソースまたは解析境界を見直す |
| `conformant` だが期待と異なる | package/target、include/exclude、policy scope、SDD projection、JSONの modules/dependencies | 対象範囲や定義が誤っていれば入力定義を修正し、同一の正しい入力を最小fixtureでも誤判定するならツールを修正候補にする |
| 終了コードと `decision` の不一致、JSON schema・orderingの破損 | CLI report、終了コード契約、同一入力の再実行結果 | ツールまたは公開契約の不整合。policy/SDDの変更で隠さない |

各問題は、次の形式で報告する。

- 事象: コマンド、終了コード、`decision`、主要なissue/violation。
- 原因分類: `tool`、`input-definition`、`target-source`、`environment`、`undetermined` のいずれか。
- 調査先: 実行ファイル、解析コンテキスト、policy/SDD、対象ソースの具体的なパス。
- 修正対象: ツール、入力定義、対象ソース、実行指定のいずれか。まだ確定できない場合は無理に修正しない。
- 再検証: 同じ解析コンテキストでの再実行、または原因を分離する最小fixtureと回帰テスト。

`violations` が空であること、issue件数が減ったこと、`--exclude` で問題が消えたことだけを修正完了の根拠にしない。

## 出力/完了条件

- 実行した解析コンテキストを package、target、target triple、analysis profile とともに示す。
- 使用したpolicy入力をYAMLまたはSDDとして示す。SDD入力では3つの正本パスを示し、派生policyを永続成果物として作らない。
- `non-conformant` では rule ID、依存元、依存先、根拠位置を示す。
- `indeterminate` では analysis issue code、理由、影響範囲を示す。
- 問題やエラーがある場合は、原因分類、調査先、修正対象、再検証方法を示す。
- 対象としたすべての解析コンテキストが `conformant` の場合だけ、検証完了とする。

## 注意

- `source-union` profile は、対象プロジェクトが非コンパイル経路を含むソース集合の解析を明示的に求める場合だけ使用する。
- `--policy` とSDD 3オプションは同時に指定しない。SDD入力は3オプションすべてを指定する。
- `--exclude` によって消えた依存は検証対象外であり、依存が存在しないことの証拠にはならない。
- 空の `violations` だけを見て適合と判断せず、必ず `decision` と `analysis_issues` を確認する。
