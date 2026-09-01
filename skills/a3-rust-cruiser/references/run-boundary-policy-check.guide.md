# YAML BoundaryPolicyによる論理モジュール境界検証ガイド

## 目的

- プロジェクトが所有するYAML BoundaryPolicyを使い、1つのCargo packageとcompilation targetの論理モジュール境界を検証する。
- SDD入力を必要とせず、BoundaryPolicyのルールを直接評価する。

## 前提条件

- `a3-rust-cruiser` コマンドが利用できること。利用できない場合はセットアップを自動実行せず、不足を報告する。
- 対象プロジェクトが所有するBoundaryPolicyのパスを確認できること。入力を推測または新規作成せず、確認できない場合は停止する。
- project root、Cargo package、compilation target、target tripleを対象プロジェクトとRust toolchainから確認できること。

## 入力選択

- `--policy <boundary-policy-path>`を境界入力として選択する。
- `--resolved-implementation-attribution`とは相互排他であり、同じ実行で併用しない。
- BoundaryPolicyはSDDに依存しない標準入力として扱う。ownerのDSLを解釈したり、別の入力へ変換したりしない。

## 検証フロー

1. 対象プロジェクトからproject root、BoundaryPolicy、package、compilation targetを確認する。
2. 対象toolchainからtarget tripleを確認する。
3. 既定の`compilation` profileで、機械判定が必要な場合はJSONを選び、次のコマンドを実行する。

   ```shell
   a3-rust-cruiser check <project-root> --policy <boundary-policy-path> --package <package> --target <target-kind>:<target-name> --target-triple <target-triple> --format json
   ```

4. 対象プロジェクトがfeature、cfg、解析範囲を明示している場合だけ、対応する`--feature`、`--cfg`、`--include`、`--exclude`を追加する。
5. 複数のpackageまたはtargetが対象なら、解析コンテキストごとに独立して実行する。

## YAML入力の確認

- BoundaryPolicyのrule ID、重大度、rule種別、論理モジュールpatternを確認する。
- `allowed`と`forbidden`は`from`と`to`、`circular`は`modules`を使う。入力の妥当性を検証できない場合は解析を開始せず入力不足または入力エラーとして扱う。
- YAML入力へ解析コンテキストや解析結果を追加しない。

## 判定と完了

- 実行後の`decision`、`violations`、`analysis_issues`、解析コンテキストの判定は、共通判定ガイドに従う。
- 対象としたすべての解析コンテキストが`conformant`の場合だけ、検証完了とする。
- `--exclude`で消えた依存は、依存が存在しない証拠として扱わない。

参照: references/run-logical-module-boundary-check.guide.md

## 注意

- BoundaryPolicyの新規設計や修正はこのガイドの対象外とする。
- `source-union` profileを使う場合も、対象プロジェクトが非コンパイル経路の解析を明示しているときだけ指定する。
