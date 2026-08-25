---
name: skills-deploy
description: a3-rust-cruiser リポジトリが所有する公開 Agent Skill の構成を検証し、プロジェクトローカルの `.env` で指定されたスキルルートへ差分確認後にデプロイするときに使う。
---

# skills-deploy / SKILL

<!-- skill-template: single-purpose -->

## 目的

- このリポジトリの `skills` を公開スキルソースとして検証し、`.env` で指定された承認済みのデプロイ先へ反映する。

## 原則

- 公開スキルの内容検証は skill-maintenance スキルへ委譲する。
- スキルの探索、差分計画、反映、削除保護は project-skill-deploy スキルへ委譲し、このスキルでは再定義しない。
- デプロイ元とデプロイ先には、実行前に解決した絶対パスを使用する。
- デプロイ先はプロジェクトルートの `.env` にある `A3_RUST_CRUISER_SKILLS_TARGET_ROOT` だけから取得し、他の環境変数や既定のホームディレクトリから推測しない。

## 関連スキル

- 必須: project-skill-deploy スキル - 公開スキルの探索、差分計画、反映、削除保護を所有するため
- 必須: skill-maintenance スキル - 公開スキルの文書構造と公開契約の検証を所有するため

## 注意

- デプロイ先への書き込みは、このスキルを明示的に実行した依頼の範囲として扱い、都度の追加承認を求めずに実行する。
- `.env` はローカル用途に限定してGit管理外とし、シークレットを記録しない。
- `.env` をshellとして実行せず、`A3_RUST_CRUISER_SKILLS_TARGET_ROOT` の値だけを読み取る。
- `A3_RUST_CRUISER_SKILLS_TARGET_ROOT` が欠落、空、重複、相対パスの場合は停止する。
- 管理外スキルの削除、staleファイルの削除、ソースから削除されたスキルのpruneを暗黙に選択しない。
- 公開スキルの配布対象判定と予約領域の除外は project-skill-deploy スキルに従う。
- CLIのローカルインストール、Git操作、GitHub Releaseへの公開は扱わない。

## フロー

### 公開スキルのデプロイ

1. Gitのリポジトリルートを解決し、その直下の `skills` をデプロイ元として確定する。
2. プロジェクトルートの `.env` から `A3_RUST_CRUISER_SKILLS_TARGET_ROOT` を1件だけ読み取り、絶対パスのデプロイ先として解決する。
3. デプロイ元とデプロイ先が同一または相互の配下でないことを確認する。
4. project-skill-deploy スキルで選択可能な公開スキルを一覧し、デプロイ対象を確定する。
5. skill-maintenance スキルで対象スキルの文書構造と公開契約を検証する。
6. project-skill-deploy スキルでdry-runし、更新、stale、管理外、削除候補を区別して利用者へ示す。
7. デプロイ先への書き込みと削除候補について承認を得た後、dry-runと同じ入力で反映する。
8. 同じ入力で再度dry-runし、対象スキルに更新またはstaleが残っていない場合にデプロイ完了とする。管理外スキルの報告は未反映差分として扱わない。
