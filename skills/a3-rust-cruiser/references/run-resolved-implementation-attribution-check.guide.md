# resolved-implementation-attributionによる論理モジュール境界検証ガイド

## 目的

- ownerが生成したresolved-implementation-attributionを直接使い、1つのCargo packageとcompilation targetの論理モジュール境界を検証する。
- ownerの解決済み帰属と、CLIが観測したRust sourceを結合したcoverageを確認する。

## 責務境界

- `sdd-designs`がcurrent artifact locator／familyの定義・選択規則とresolved-implementation-attributionの導出意味を所有する。
- `a3-lint`がcurrent artifact instanceの解決、owner family validation、ownerの`projection materialize` operationのdispatch／実行を行い、一時出力と`resolved_output_path`を返す。
- `a3-lint`の導入や更新はa3-lint側へ委譲し、このガイドでインストールやversion選択を行わない。
- `a3-rust-cruiser`は返された一時出力を変換せずに読み込み、Rust sourceの観測、解析コンテキストの適用、境界判定とレポートを所有する。

## 前提条件

- `a3-rust-cruiser` コマンドが利用できること。利用できない場合はセットアップを自動実行せず、不足を報告する。
- 対象プロジェクトにcurrent SDD design familyがあり、この入力経路を使うことを選択していること。ない場合はreferences/run-boundary-policy-check.guide.mdへ切り替える。
- `a3-lint`とownerのruntime operationを利用できること。利用できない場合は、生成を試みず不足または未対応として停止する。
- project root、Cargo package、compilation target、target tripleを対象プロジェクトとRust toolchainから確認できること。

## 入力生成の委譲

1. 対象プロジェクトのrootを指定して、ownerのmaterialize operationを実行する。

   ```shell
   a3-lint sdd projection materialize --root <project-root> --format json
   ```

2. 出力の`status`が`materialized`であることと、`resolved_output_path`が返され、そのパスにファイルが存在することを確認する。`--format text`では受け渡すパスが表示されないため、機械的に引き継ぐ場合はJSONを使う。
3. `resolved_output_path`の値をそのまま`--resolved-implementation-attribution`へ渡す。パスを組み立てたり、globで探索したり、ownerのraw DSLから手作業で生成したりしない。
4. 出力はruntime管理の一時成果物として扱い、編集・コミットしない。

## 入力選択

- `--resolved-implementation-attribution <resolved-implementation-attribution-path>`を境界入力として選択する。
- `--policy`とは相互排他であり、同じ実行で併用しない。
- ownerのraw DSLを解釈せず、ownerが確定したfile attributionと依存関係を中間変換なしで使用する。

## 検証フロー

1. 対象プロジェクトからproject root、package、compilation targetを確認する。
2. 「入力生成の委譲」に従って`resolved_output_path`を取得する。
3. 対象toolchainからtarget tripleを確認する。
4. 機械判定が必要な場合はJSONを選び、次のコマンドを実行する。

   ```shell
   a3-rust-cruiser check <project-root> --resolved-implementation-attribution <resolved-implementation-attribution-path> --package <package> --target <target-kind>:<target-name> --target-triple <target-triple> --format json
   ```

5. 対象プロジェクトがfeature、cfg、解析範囲を明示している場合だけ、対応する`--feature`、`--cfg`、`--include`、`--exclude`を追加する。
6. 複数のpackageまたはtargetが対象なら、解析コンテキストごとに独立して実行する。

## 生成失敗時

- `a3-lint`の終了コードが0でない、JSONの`command_outcome.kind`が`success`でない、`status`が`materialized`でない、`resolved_output_path`がない、またはそのパスにファイルがない場合は、a3-rust-cruiserの判定を開始せず停止する。
- JSONに`phase_error`がある場合は`code`、`reason_code`、`message`を記録し、runtime、surface、familyの不足または実行エラーを`a3-lint`側へ委譲する。
- YAML BoundaryPolicyは、SDD経路が適用対象外または利用者が明示的に選択した場合に入力選択から切り替える。SDD生成失敗の代替として切り替えない。

## resolved入力の確認

- JSONレポートの`schema_version`が`3`であることを確認する。
- `normalized_sdd_coverage`の`covered_modules`、`unresolved_modules`、`source_provenance`を確認する。
- CLIの解析コンテキスト、coverage、Rust module IDを入力ファイルへ追加しない。
- 観測されたsource fileの帰属がない場合、または同一moduleのsource file群が異なるowner tupleへ解決される場合は、違反なしへ読み替えず`indeterminate`として扱う。

## 判定と完了

- 実行後の`decision`、`violations`、`analysis_issues`、解析コンテキストの判定は、共通判定ガイドに従う。
- coverageが全て解決していることだけで境界適合とは判定せず、`violations`と`decision`を確認する。
- 対象としたすべての解析コンテキストが`conformant`の場合だけ、検証完了とする。

参照: references/run-logical-module-boundary-check.guide.md

## 注意

- resolved-implementation-attributionの導出規則、current artifact locator／familyの定義・選択規則、owner DSLの意味解釈は`sdd-designs`へ委譲し、このガイドで再定義しない。
- current artifact instanceの解決、owner family validation、runtime operationのdispatchと一時出力のmaterializeは`a3-lint`へ委譲し、このガイドで別の生成経路を作らない。
- `source-union` profileを使う場合も、対象プロジェクトが非コンパイル経路の解析を明示しているときだけ指定する。
