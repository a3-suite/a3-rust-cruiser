# GitHub Releaseセットアップガイド

## 目的

- GitHub Releaseの固定版assetを検証し、利用者が指定したdirectoryへa3-rust-cruiserを導入する。
- 導入後のversionとcommand pathを確認し、境界検証を実行できる状態にする。

## 前提条件

- 対象versionを`0.1.0`のような完全なrelease versionとして確定していること。
- install directoryが存在する書き込み可能な絶対パスで、symlinkではなく、`PATH`から同じdirectoryへ解決できること。
- GitHubからの取得とinstall directoryへの書き込みについて、利用者の承認を得ていること。

## セットアップフロー

1. `uname`でplatformを確認し、Linux x86_64またはmacOS arm64以外では停止する。
2. 既存の`a3-rust-cruiser`とinstall directory内の同名fileを確認する。
3. 次のコマンドを実行する。

   ```shell
   bash {skill-root}/scripts/install-release.sh --version <version> --install-dir <install-dir>
   ```

4. 異なる既存CLIがある場合はinstallerを停止する。置換が必要なら、既存fileの移動または削除を別操作として利用者へ提示し、承認後に実施してからinstallerを再実行する。
5. 出力された実行file、version、command pathを確認する。
6. 対象プロジェクトの入力経路を選び、対応する入力経路別ガイドに従って検証する。
   - YAML BoundaryPolicy: references/run-boundary-policy-check.guide.md
   - resolved-implementation-attribution: references/run-resolved-implementation-attribution-check.guide.md

## 検証と配置

- release tag、asset名、versionは同じ固定versionから導出する。
- assetと同じreleaseの`SHA256SUMS`を取得し、対象assetのSHA-256を照合する。
- archiveが単一の通常file `a3-rust-cruiser`だけを含むことを確認する。
- 展開した実行fileの`--version`が要求versionと一致した後だけinstall directoryへ配置する。
- 同一内容が導入済みなら成功とし、異なる既存fileは変更しない。
- 配置中は確認済みinstall directoryを作業directoryとして固定し、既存pathを上書きしないhard linkで未配置の宛先だけを有効化する。
- hard link commandが副作用後に失敗を返した場合は、候補と宛先の実file identityが一致するときだけ配置完了として継続する。
- installerは既存fileの削除、移動、上書き、退避、復元を行わない。

## 失敗対応

- download、checksum、archive、version検証の失敗ではinstall directoryの実行fileを変更しない。
- install directoryが`PATH`に無い、別のCLIにshadowされる、または同時installのlockを取得できない場合は配置前に停止する。
- 配置後のversionまたはcommand path確認に失敗した場合は、成功として扱わない。
- 配置直前に宛先が作成された場合は、そのfileを変更せず停止する。
- install directoryのpath identityが配置中に変化した場合は、確認済みdirectory内に保持された実行fileのpathを標準エラーへ示す。

## 注意

- shell profile、Rust toolchain、Cargo、boundary policyは変更しない。
- `latest`、branch、source build、checksum省略による導入は扱わない。
- 異なる既存fileの置換はinstallerの非対象とする。
