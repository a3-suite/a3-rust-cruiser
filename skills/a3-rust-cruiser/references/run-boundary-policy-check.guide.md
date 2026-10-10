# YAML BoundaryPolicyによる論理モジュール境界検証ガイド

## 目的

- プロジェクトが所有するYAML BoundaryPolicyを使い、1つのCargo packageとcompilation targetの論理モジュール境界を検証する。
- SDD入力を必要とせず、BoundaryPolicyのルールを直接評価する。

## 前提条件

- `a3-rust-cruiser` コマンドが利用できること。利用できない場合はセットアップを自動実行せず、不足を報告する。
- 対象プロジェクトが所有するBoundaryPolicyのパスを確認できること。ない場合、作成の依頼も受けていればreferences/create-boundary-policy.guide.mdへ進む。検証だけの依頼では入力不足として停止する。
- project root、Cargo package、compilation target、target tripleを対象プロジェクトとRust toolchainから確認できること。

## 入力選択

- `--policy <boundary-policy-path>`を境界入力として選択する。
- `--resolved-implementation-attribution`とは相互排他であり、同じ実行で併用しない。
- この経路の適用対象は、スキルの原則に従う非SDDプロジェクトとする。ownerのDSLを解釈したり、別の入力へ変換したりしない。

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
- YAMLの記述方法はreferences/create-boundary-policy.guide.mdを参照する。入力の妥当性を確認できない場合は入力不足として停止し、CLIが返した入力エラーは境界判定と区別する。
- YAML入力へ解析コンテキストや解析結果を追加しない。

## 判定と完了

- 実行後の`decision`、`violations`、`analysis_issues`、解析コンテキストの判定は、共通判定ガイドに従う。
- 対象としたすべての解析コンテキストが`conformant`の場合だけ、検証完了とする。
- `--exclude`で消えた依存は、依存が存在しない証拠として扱わない。

参照: references/run-logical-module-boundary-check.guide.md

## 注意

- BoundaryPolicyの作成・修正はreferences/create-boundary-policy.guide.mdへ委譲する。
- `source-union` profileを使う場合も、対象プロジェクトが非コンパイル経路の解析を明示しているときだけ指定する。
