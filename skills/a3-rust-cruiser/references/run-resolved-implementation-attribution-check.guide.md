# resolved-implementation-attributionによる論理モジュール境界検証ガイド

## 目的

- ownerが生成したresolved-implementation-attributionを直接使い、1つのCargo packageとcompilation targetの論理モジュール境界を検証する。
- ownerの解決済み帰属と、CLIが観測したRust sourceを結合したcoverageを確認する。

## 責務境界

- `sdd-designs`がcurrent artifact locator／familyの定義・選択規則とresolved-implementation-attributionの導出意味を所有する。
- `a3-sdd`がSDD runtimeの公開導線と出力方法を所有する。このガイドではruntimeの操作や生成手順を再定義しない。
- `a3-rust-cruiser`は返された一時出力を変換せずに読み込み、Rust sourceの観測、解析コンテキストの適用、境界判定とレポートを所有する。

## 前提条件

- 共通判定ガイドの前提条件を満たすこと。
- 対象のSDDプロジェクトにcurrent SDD design familyがあること。不足している場合は入力不足として停止する。
- `a3-sdd`の正規導線で生成されたresolved-implementation-attributionを利用できること。利用できない場合は、生成手順をこのガイドで補完せず停止する。
- project root、Cargo package、compilation target、target tripleを対象プロジェクトとRust toolchainから確認できること。

## 入力契約

- 入力物は、`a3-sdd`の正規導線が出力する`name: resolved-implementation-attribution`、`version: 1`の成果物に限定する。
- 入力物のshape、provenance、帰属、依存関係の意味と検証規則は、`a3-sdd`／`sdd-designs`の正本に従う。

## 入力物の受け渡し

1. `a3-sdd`の正規導線で生成されたresolved-implementation-attributionのパスを受け取る。
2. 受け取ったパスをそのまま`--resolved-implementation-attribution`へ渡す。パスを組み立てたり、globで探索したり、ownerのraw DSLから手作業で生成したりしない。
3. 出力はruntime管理の一時成果物として扱い、編集・コミットしない。

## 入力選択

- `--resolved-implementation-attribution <resolved-implementation-attribution-path>`を境界入力として選択する。

## 検証フロー

1. 対象プロジェクトからproject root、package、compilation targetを確認する。
2. 「入力物の受け渡し」に従ってresolved-implementation-attributionのパスを確定する。
3. 対象toolchainからtarget tripleを確認する。
4. 機械判定が必要な場合はJSONを選び、次のコマンドを実行する。

   ```shell
   a3-rust-cruiser check <project-root> --resolved-implementation-attribution <resolved-implementation-attribution-path> --package <package> --target <target-kind>:<target-name> --target-triple <target-triple> --format json
   ```

5. 対象プロジェクトがfeature、cfg、解析範囲を明示している場合だけ、対応する`--feature`、`--cfg`、`--include`、`--exclude`を追加する。
6. 複数のpackageまたはtargetが対象なら、解析コンテキストごとに独立して実行する。

## 入力物が利用できない場合

- 受け取ったパスがない、ファイルを読み取れない、または入力契約に適合しない場合は、a3-rust-cruiserの判定を開始せず入力不備として停止する。
- `a3-sdd`のruntime不足や生成失敗の詳細はこのガイドで再分類せず、`a3-sdd`側の出力と診断へ委譲する。
- SDDプロジェクトではYAML BoundaryPolicyへ切り替えない。入力不足や生成失敗はowner側で解消する。

## resolved入力の確認

- JSONレポートの`schema_version`が`3`であることを確認する。
- `normalized_sdd_coverage`の`covered_modules`、`unresolved_modules`、`source_provenance`を確認する。
- CLIの解析コンテキスト、coverage、Rust module IDを入力ファイルへ追加しない。
- 観測されたsource fileの帰属がない場合、または同一moduleのsource file群が異なるowner tupleへ解決される場合は、違反なしへ読み替えず`indeterminate`として扱う。

## 判定と完了

- 実行後の`decision`、`violations`、`analysis_issues`、解析コンテキストの判定と完了条件は、共通判定ガイドに従う。
- coverageが全て解決していることだけで境界適合とは判定せず、`violations`と`decision`を確認する。

参照: references/run-logical-module-boundary-check.guide.md
