# Everything this app says, in Japanese.
#
# Yume 2kki is a Japanese game and the wiki publishes a Japanese name for nearly every world, so
# this is the language most of the graph is already written in: see `World::title_jp`.
#
# A message left out here is read from `en-US` instead, so this file may be short of that one
# without anything going blank. What it may not do is disagree with it about what a message is
# named or which values it asks for.

language-name = 日本語
language = 言語

## The frame before the graph.

dump-loading = 読み込み中...
dump-failed = 世界のデータを読み込めませんでした。

# 世界のデータを組み立てているサーバーが、いま何をしているか。`world::building` を参照。
dump-task-changes = wikiの更新を確認しています…
dump-task-worlds = 世界を読み込んでいます…
dump-task-connections = 世界のつながりを読み込んでいます…
dump-task-assembling = グラフを組み立てています…

## The sidebar and its tabs.

tab-worlds = マップ
tab-authors = ツクラー
tab-versions = バージョン
hide-sidebar = サイドバーを隠す
show-sidebar = サイドバーを表示

## The graph tab.

fps = { $fps } fps
graph-size = { $worlds } マップ、{ $connections } 接続
layered = 階層表示
layered-hint = マップを深さごとの層に分けます
search-worlds = マップを検索 (英名で検索可能)
search-authors = ツクラーを検索
search-versions = バージョンを検索

worlds = { $count } マップ

showing-authors = { $total } 名
showing-authors-cut = { $total } 名中 { $shown } 名
showing-versions = { $total } 件のバージョン
showing-versions-cut = { $total } 件中 { $shown } 件のバージョン

world-author = ツクラー
world-author-hint = ツクラーのマップをすべて表示
world-map-hint = 地図を見る
world-move-up = 真上のマップを表示
world-connections = 接続 { $count }本、
world-descendants = 後続 { $count }マップ
dead-end = 行き止まり
junction = 分岐点

nothing-selected = マップをクリックすると原点までの経路をたどります。右クリックで詳細を表示します。

untaken-worlds = ヒントを見る
untaken-worlds-hint = まだ訪れていない場所につながるマップを、もう一度訪れてみましょう。

## The route home.

route-length = 原点から { $count } 接続
path-length = { $origin } から { $count } 接続
no-path = ここへの道はありません。
way-length = { $count } 接続
directions-title = { $origin } → { $destination }
way-via = { $world } 経由
zoom-in-world = マップを中心に
zoom-out-route = 経路全体を表示
trace-route = このマップへの経路をたどる

## Ways on from a world.

no-forward-connections = 降下接続はありません。
forward-connections = 降下接続 { $count }本

## What hangs off a world.

no-notable-descendants = 主要な後続マップはありません。
notable-descendants = 主要な後続マップ:
notable-world = { $title } ({ $kind }、接続 { $degree })

## The catalogs.

author-row = { $name } ({ $worlds })
version-row = { $name } ({ $worlds })
version-row-dated = { $name } ({ $worlds }, { $released })
version-released = { $released } 実装
version-added = { $worlds } 追加
layer-depth = 深さ { $depth }

## The menu a right-click opens.

menu-descendants = 後続マップを強調
menu-directions-to = ここへの経路
menu-directions-to-hint = { $world } からここまでの経路を表示
menu-directions-from = ここからの経路
menu-directions-from-hint = ここから { $world } までの経路を表示
menu-open-wiki = wikiで見る

## The rocker in the corner.

rocker-shallower = 浅く
rocker-deeper = 深く

## The settings tab.

hub-push = ハブの反発
hub-push-hint = 値が大きいほど、接続の多いマップが周囲から遠ざかります
link-reach = 接続の長さ
link-reach-hint = つながった2つのマップが離れられる距離を階層数で表します。一方通行の接続はこの制限を受けません
ui-scale = UIの大きさ
ui-scale-hint = パネルと文字の大きさ
antialias = 輪郭をなめらかに
antialias-hint = グラフの輪郭をなめらかにします。フレームレートがディスプレイに届かないときは、
    まずこれを切ってください。
antialias-restart = 次回の起動から反映されます。

leaning = 指したマップへ寄る
leaning-hint = 一覧でマップを指すと、視点がゆっくりとそこへ寄ります。オフにすると視点は動きません。
show-controls = 操作方法を表示
clear-cache = ダウンロードを消去
clear-cache-hint = 起動をまたいで保存したマップの画像を消去します。次に見るときは取得し直します
clear-cache-clearing = 消去中...
clear-cache-done = ダウンロードを消去しました
clear-cache-failed = ダウンロードを消去できませんでした
update-check = 更新を確認
update-check-hint = このビルドより新しいリリースがあるかGitHubに問い合わせます
update-checking = 確認中...
update-current = これが最新のリリースです
update-ready = { $version } が利用できます
update-install = インストール
update-installing = ダウンロード中...
update-installed = インストールしました。次回の起動から反映されます。
update-failed = 更新を取得できませんでした
stamp = { $year }/{ $month }/{ $day } { $hour }:{ $minute }

last-update = データ更新: { $when }
last-update-hint = データをwikiから読み取った日時
last-full-update = 全体の再取得: { $when }
last-full-update-hint = 全体の再取得では、wikiから最新のデータを読み直し、名前が変わったマップが残した空きを詰めます。

## YNOproject にログインして、行ったことのある世界だけを描く。

yno = YNOprojectでの探検記録
yno-loading = 記録を読み込み中...
yno-hint = YNOprojectにログインすると、探検記録をこのグラフに表示できます。
yno-user = ユーザー名
yno-password = パスワード
yno-sign-in = ログイン
yno-sign-out = ログアウト
yno-working = ログイン中...
yno-signed-out = セッションの有効期限が切れました。もう一度ログインしてください。
yno-signed-in = ログインしました。
yno-completion = { $seen } / { $worlds }（{ $percent }%）
yno-completion-hint = 発見したマップの割合です。
yno-refresh = 更新
yno-refresh-hint = 訪問済みのマップをYNOprojectから再度読み込みます。
yno-promise = ユーザー名とパスワードは、YNOprojectでの探検記録を読み取るためだけに使います。yumezuがそれらを他人に共有することも、アカウントを変更することもありません。
yno-source = アカウントの扱いについて
menu-reveal = 【デバグ】訪問済みとみなす
menu-reveal-hint = 更新ボタンで新しいマップが増えた状態を再現します。
frontier = 探検者モード
frontier-hint = 訪問済みのマップだけを表示します。そこから1つ先のマップは未訪問の場所として表示します。
unvisited-location = 未訪問の場所
github-link = GitHubで見る
download-for = {$platform}版をダウンロード

## The controls, named on the first run.

guide-title = 操作方法
guide-inputs = 入力
guide-fly-action = 前後に移動
guide-strafe-action = 左右に移動
guide-orbit-mouse-input = 左クリック
guide-orbit-mouse-action = 視点回転
guide-orbit-touch-input = 指1本
guide-orbit-touch-action = 視点回転
guide-options-input = 右クリック
guide-options-action = メニュー
guide-pan-input = 右クリック（長押し）
guide-pan-action = 平行移動
guide-pinch-input = 指2本
guide-pinch-action = 拡大縮小・平行移動
guide-scroll-input = ホイール
guide-scroll-action = 拡大縮小
guide-rocker = 深さスイッチ
guide-rocker-body = 右下の2つの矢印は、グラフの層をまとめて選びます。
guide-got-it = わかった
dont-show-again = 今後は表示しない

## The app, offered to a page whose browser has a package to install.

download-app = {$platform}アプリを入手

## The wiki's maps.

map-none = 海外wikiにはこのマップの地図が掲載されていません。
map-missing = 地図の画像を読み込めません。
map-fit = マップ全体をウィンドウに収める
map-maximize = ウィンドウを画面いっぱいに広げる
map-restore = ウィンドウを元の大きさに戻す

## What a connection asks of a player walking it.

gate-effect = エフェクトが必要
gate-chance = 確率あり
gate-seasonal = 季節限定
gate-locked = 反対側の入口から解除
gate-locked-condition = 条件付きで解除
gate-exit-point = ショートカットの出口から逆走
gate-dead-end = 反対側の孤立エリアからのみ
gate-isolated = 孤立エリアへ通じる

gate-effect-detail = { $effects } が必要
gate-chance-detail = 確率 { $chance }
gate-seasonal-detail = { $season ->
        [Spring] 春のみ
        [Summer] 夏のみ
        [Fall] 秋のみ
        [Winter] 冬のみ
       *[other] { $season } のみ
    }

## Which ways round a connection can be walked.

walk-freely = 自由に通行
walk-free-both = 両方通行です。
walk-one-way = 一方通行です。
walk-no-entry = ここからは入れません。
walk-none = 現在は通行できません。
walk-dead-end = メインエリアからは入れません。
walk-isolated = メインエリアにつながりません。
walk-locked-out = 反対側の入口から解除できます。
walk-locked-back = 反対側の入口からこの領域への通行を解除します。
walk-both =
    ここから: { $out }
    ここへ: { $back }。
walk-out-only = ここからのみ: { $out }
walk-back-only = ここへのみ: { $back }
