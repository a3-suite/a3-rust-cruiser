# 回帰テスト報告

## 判定

✅ 修正後の回帰ゲートを満たす。Cargo lint 警告を解消し、契約テストを適切な統合層へ集約した。C1・行カバレッジ、Rust 全 target、CI 契約 shell、installer、smoke を確認した。報告作成時点でコミット・push・deploy は未実施。

## lint・軽量ゲート

- ✅ `A3_SKILLS_ROOT=$A3_SKILLS_ROOT a3-lint lint ... src tests`: 54 targets、終了0。
- ✅ 同 lint の Rust/Cargo framework: 54 targets、診断0、終了0。
- ✅ Markdown lint（README、公開 skill、references）: 7 targets、終了0。最終証跡（本報告と archive issue）も2 targets、終了0。
- ✅ `cargo fmt --all -- --check`、`cargo clippy --locked --all-targets --all-features -- -D warnings`、`git diff --check`。
- lint は今回の Rust/test 修正後に再実行した。rule の無効化、閾値変更、除外追加、新規依存はない。

## 内部依存チェック

- 対象リポジトリの CI・local-install・公開文書に、自己依存検証を実行する宣言済みコマンドはないため対象外。
- 設計上の依存 SSOT は `sdd/dsl/designs/scopes/logical-module-boundary-validation/logical-structure.sdd.yml`。architecture alignment 2件は既存の設計参照整合であり、内部依存解析の代替とは扱わない。

## 回帰範囲

- 回帰方式: full regression。Rust 実装・契約テスト変更のため applicable。
- 今回の変更集合: `src/command_interface/cli.rs`、`src/boundary_policy_input/normalized_sdd.rs`、`src/rust_dependency_observation/cfg_context.rs`、`tests/contracts/logical_module_boundary_validation_integration_test/{boundary_policy_input.rs,cli_outcome.rs,dependency_graph.rs,validation_report.rs}`、`sdd/dsl/specs/contract-core/subjects/logical-module-boundary-validation/test-maps.sdd.yml`、本報告、`.a3-skills/local-issue/archive/issue_20260908_102414_regression-gates.md`。
- 既存変更として除外したもの: README、SDD core 2件、公開 skill と既存 guide 3件、既存ログ。HEAD `69154e7de8bfbf00bd67cdbb98c69dab9e1d42a9` 基準で src/tests の修正は今回のものだけ。
- 契約・SSOT・責務境界・閾値・除外・依存・外部 `a3-sdd`・deploy は変更していない。
- 対象契約: `subject.logical-module-boundary.validation`。E2E はプロジェクトの test-map が `none` を明示するため対象外。

## テスト結果

- ✅ Rust unit: 59 passed / 0 failed / 0 skipped。
- ✅ Rust integration: architecture alignment 2、check command 1、working-directory 1、logical-module-boundary contract 152、計156 passed。
- ✅ Rust 全 target: `cargo test --locked --all-targets --all-features`、215 passed / 0 failed。
- ✅ CI 契約 shell: runner trust、tool version、release publication、release authority、release workflow。
- ✅ 公開 installer shell: install-release tests passed。
- ✅ minimal-project smoke: conformant、終了0。

## カバレッジ

- 実行単位: `cargo +nightly-2026-05-12 llvm-cov --locked --all-targets --all-features --branch --json`（unit と integration を同時実行）。
- 計測証拠: `tests/tmp/coverage/fix-20260908.IDjvRT/coverage.json`、SHA256 `fb4653b4889be51f96be5d367b06add94b103a11d5cc1c68b088bee18aeaf7e5`。
- C0: 別個の statement 指標がなく計測不能。line/region へ読み替えていない。
- C1: 999/1232 = **81.09%**（閾値80%、合格）。
- line: 7045/7448 = **94.59%**（閾値90%、合格）。
- 契約未カバー懸念: なし。未カバー候補は下記の通り個別に no-action または今回の契約テストで解消した。

### 個別候補分析

- `cargo_project.rs`: C1 76/82=92.68%、line 488/552=88.41%。デフォルト feature の optional dependency、test target の exact dev-dependency を契約統合テストで追加観測した。残る低頻度分岐は hyphen/underscore feature alias の同義経路、非選択 target、前段で保証済みの manifest shape 再検査で、公開結果の意味を増やさず個別に埋めるのは no-action。
- `cfg_context.rs`: C1 44/50=88.00%、line 248/258=96.12%。source-union の nested `cfg_attr(path)` と malformed cfg metadata の三値 fail-closed を追加観測した。残りは parser が返す Unknown を保つ防御枝で、契約上の未確認はない。
- `path_resolution.rs`: C1 85/112=75.89%、line 412/437=94.28%。公開 re-export 循環を追加し、通常 alias/glob/re-export、private item、super 解決は既存統合で確認した。残る empty path、深度上限、glob の空結果、visibility fallback は fail-closed の防御枝であり no-action。
- `macro_safety.rs`: C1 85/108=78.70%、line 277/298=92.95%。既存の invalid local macro、shadowing、cfg guarded、exact validation macro の契約ケースで安全側判定を確認済み。未カバーは token-shape 拒否や同じ fail-closed guard の組合せで、追加は no-action。
- `macro_syntax.rs`: C1 21/28=75.00%、line 199/202=98.51%。既存の multi-arm、invalid runtime macro、unsupported shape が false/indeterminate を確認済み。単一 rule 分離・semicolon・metavariable の残りは内部 parser fallback で、公開契約の観測不足ではないため no-action。
- `visitor.rs`: C1 167/232=71.98%、line 689/781=88.22%。nested JSON の array/map と `matches!` guard を異なる3依存先へ向け、各 path-reference を個別に契約レベルで確認した。残る visitor dispatch の未使用 AST variant、invalid token fallback、scope restore 枝は supported contract 外または既存 fail-closed 観測と重複するため no-action。

## 修正内容

- `normalized_sdd` の実ファイルを使う結合 unit test を削除し、owner-resolved attribution の多スコープ・5 artifact・scope edge を既存 CLI 契約統合テストへ移した。実依存の有無で判定が切り替わること、covered modules と artifact 数を公開結果で固定している。
- `cli::run` の出力を private `write_result` に集約し、`writeln!` 後の `flush` 失敗を終了コード3として扱う実装欠陥を修正した。private 実装を直接起動せず、閉じた Unix 出力 endpoint を使う CLI 契約統合テストで write failure・診断・終了コード3を確認した。
- Cargo feature/target dependency、cfg、macro/path/re-export の不足観測を既存契約ファイルへ追加した。
- SDD owner の traceability 指摘に対応し、追加6件を既存 verification/case と check_refs へ `test-maps.sdd.yml` で紐付けた。契約 DSL の意味、閾値、除外は変更していない。

## SDD traceability

- staged scope 監査で、追加した統合テスト6件に test map の symbol/contract mapping が不足していることを確認した。
- `test-maps.sdd.yml` に resolved attribution、CLI write failure、re-export、feature/target dependency、nested JSON/matches、source-union cfg_attr の各テストを追加し、既存 verification/case の check_refs へ接続した。
- 2026-09-08T11:28:49+09:00 の owner 再監査で6件すべての実体/map symbol一致と check_refs の有効性を確認し、停止 finding なし・SDD projection `PASS` となった。

## レビュー

- 主レビュー: diff、変更境界、既存契約・SSOT・閾値・除外を確認。今回の7つの src/tests ファイルと証跡だけを採用し、既存の7つの公開文書/SDD変更は除外した。指摘なし。
- レビュー担当判定: 単一責務・単一契約は不充足、SSOT/意味/境界不変は充足、既存変更からの分離は充足、独立保証の追加価値が小さいは不充足。したがって独立レビューを実施した。
- 独立レビュー: `/root/fix_independent_review`（sub-agent、実行モデル識別子は取得不可、変更・テスト実行なし）。修正後の固定変更集合を再確認し、medium finding 2件と移管観測不足・AAA不備の追加候補がすべて解消、残存 finding なし。
- 独立レビューの未確認: stdout endpoint の検証は Unix 限定で、write成功後に flush だけが失敗する経路は実装レビューで確認した。現行プロジェクトの対象環境では追加修正不要。

## ボトルネック

最長は専用 target の clean coverage build（約6秒）。テスト本体は約0.7秒で、timeout や反復実行が原因ではない。

## コミット

- 必須ゲート: ✅ lint、format、clippy、Rust 全 target、C1、line、shell、installer、smoke。
- 報告作成時点で commit/push/deploy は未実施。現在のコミット状態は Git の履歴と確認表示を正本とする。

## エラー・警告

- エラーなし。
- `llvm-cov --branch` の unstable 警告と C0 計測不能は tool/metric の制約として記録し、合否を改変していない。
