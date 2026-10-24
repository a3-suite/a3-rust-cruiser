---
created_at: "2026-09-08T10:24:14+09:00"
updated_at: "2026-09-08T11:23:58+09:00"
status: "closed"
summary: "契約テストの責務整理と未検証分岐の解消により回帰ゲートを回復する"
---
# 回帰ゲートの回復

## 概要

直前の回帰結果を対象とする。既存の README、SDD core、公開スキルの変更は対象外。

## 期待状態

既存契約を維持したまま、適切なテスト層で必要な分岐を検証できる。

## 現状

209 tests passed。Cargo lint は入力処理の結合を確認する unit 1 件で警告。C1 は 78.409%、行は 93.491%。詳細は logs/agents/20260908/20260908_101322_regression-test.md。

## 解決すべき問題点

回帰ゲートが未達。対象は normalized_sdd の警告テストと、既存契約に対応する未カバー分岐。前述の lint と coverage JSON で再現可能。放置すると保証不足を見逃す。重複保証は統合し、不足観点は既存契約テストへ補う。

## 根本原因分析

1. なぜゲート未達か: unit の対象呼び出し数警告と分岐カバレッジ不足がある。
2. なぜ警告になるか: prepare と finalize の結合を unit として確認している。未カバー側には stdout 失敗やマクロ内の依存など未観測ケースがある。
3. なぜ残っていたか: テスト成功件数だけでは層の適合や契約内の観測漏れを保証できない。その他の未カバーは到達性を個別確認し、追加テストの必要性を判断する。

## 完了条件

- Rust/Cargo lint、整形、clippy が成功する。
- フル回帰が成功し、C1 80%以上、行90%以上、対象個別分析に未解消がない。
- 固定した変更集合を主レビューと必要な独立レビューで確認する。

## 対応方針

単一の回帰修正単位として既存テストへ保証を集約する。契約、閾値、除外、依存、外部プロジェクト、デプロイ、コミットは変更しない。HEAD 69154e7de8bfbf00bd67cdbb98c69dab9e1d42a9 時点で src/tests は変更なし。

## 解決進捗

### 2026-09-08T10:30:00+09:00 - 初期整理

既存変更を分離した。警告の実装経路と既存統合テストを確認したため、保証を落とさず統合できるか確認してから編集する。カバレッジは未達のまま。次は契約観点を追加し、lint を先行して再測定する。

## 最新進捗

2026-09-08T11:01:07+09:00 時点で、private 実装を直接起動していた unit を削除し、resolved attribution の多スコープ・5 artifact・scope edge を公開 CLI 契約統合テストへ移管した。stdout write failure は閉じた Unix endpoint を使う公開 CLI テストで終了コード3と診断を確認した。nested JSON array/map と `matches!` guard は異なる3依存先を個別に確認する形へ修正した。

Rust unit 59件、integration 156件、全 target 215件が成功。a3-lint は Rust/any と Cargo の各54 targetで成功し、C1 999/1232=81.09%、line 7045/7448=94.59%で閾値を満たした。CI契約 shell、installer、minimal-project smoke、主レビュー、独立レビューも完了し、残存 finding はない。コミット、push、deploy は未実施。

## SDD traceability 追補

staged scope の owner 監査で、今回追加した統合テスト6件の test map symbol/contract mapping が不足していることを確認した。`sdd/dsl/specs/contract-core/subjects/logical-module-boundary-validation/test-maps.sdd.yml` に既存 verification/case と check_refs を追加し、契約 DSL の意味・閾値・除外は変更しない。

mapping 修正後の owner 再監査で6件すべての実体/map symbol一致と check_refs の有効性を確認し、停止 finding なし・SDD projection `PASS` となった。

## クローズ要約

回帰ゲート未達の原因を、テスト層の不適合と不足していた契約観測へ分離して解消した。閾値・除外・SSOT・外部プロジェクトは変更していない。独立レビューでは残存 finding なし。次に見るべき証跡は `logs/agents/20260908/20260908_014718_fix-regression-test.md` と coverage JSON であり、外部 owner の再監査や deploy は別承認の作業として残る。
