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

4. 対象プロジェクトが feature、cfg、解析範囲を明示している場合だけ、対応する `--feature`、`--cfg`、`--include`、`--exclude` を追加する。
5. 複数の package または target が対象なら、解析コンテキストごとに独立して実行する。

## 判定基準

- 終了コード `0` / `decision: conformant`: 指定した解析コンテキストでは適合。
- 終了コード `1` / `decision: non-conformant`: `violations` を根拠に境界違反または循環依存を報告する。
- 終了コード `2` / `decision: indeterminate`: `analysis_issues` と影響範囲を報告し、適合とは判定しない。
- 終了コード `3`: 引数、入力、解析、または出力処理のエラーとして標準エラーを報告する。
- 終了コードと `decision` が対応しない場合は、結果を採用せず契約不整合として報告する。

## 出力/完了条件

- 実行した解析コンテキストを package、target、target triple、analysis profile とともに示す。
- 使用したpolicy入力をYAMLまたはSDDとして示す。SDD入力では3つの正本パスを示し、派生policyを永続成果物として作らない。
- `non-conformant` では rule ID、依存元、依存先、根拠位置を示す。
- `indeterminate` では analysis issue code、理由、影響範囲を示す。
- 対象としたすべての解析コンテキストが `conformant` の場合だけ、検証完了とする。

## 注意

- `source-union` profile は、対象プロジェクトが非コンパイル経路を含むソース集合の解析を明示的に求める場合だけ使用する。
- `--policy` とSDD 3オプションは同時に指定しない。SDD入力は3オプションすべてを指定する。
- `--exclude` によって消えた依存は検証対象外であり、依存が存在しないことの証拠にはならない。
- 空の `violations` だけを見て適合と判断せず、必ず `decision` と `analysis_issues` を確認する。
