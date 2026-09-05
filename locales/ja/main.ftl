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

dump-loading = 世界を読み込んでいます…
dump-failed = 世界のデータを読み込めませんでした。

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
layered-hint = マップを深さごとの層に分ける
search-worlds = マップを検索 (英名で検索可能)
search-authors = ツクラーを検索
search-versions = バージョンを検索

# Japanese counts one and many alike, so there is one form where English has two.
worlds = { $count } マップ

showing-authors = { $total } 名
showing-authors-cut = { $total } 名中 { $shown } 名
showing-versions = { $total } 件のバージョン
showing-versions-cut = { $total } 件中 { $shown } 件のバージョン

## The selected world.

world-author = ツクラー
world-author-hint = ツクラーのマップをすべて表示
world-map-hint = 地図を見る
world-move-up = 真上のマップを表示
world-connections = 接続 { $count }本、
world-descendants = 後続 { $count }マップ
dead-end = 行き止まり
junction = 分岐点

nothing-selected = マップをクリックすると原点までの経路をたどります。右クリックで詳細を表示します。

## The route home.

route-length = 原点から { $count } 接続
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
menu-open-wiki = wikiで見る

## The rocker in the corner.

rocker-shallower = 浅く
rocker-deeper = 深く

## The settings tab.

hub-push = ハブの反発
hub-push-hint = 値が大きいほど、接続の多いマップの反発力が強くなります
ui-scale = UIの大きさ
ui-scale-hint = パネルと文字を描く大きさ
show-controls = 操作方法を表示
github-link = GitHubで見る
android-link = Android版をダウンロード

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
dont-show-again = 今後から非表示にする

## The Android app, offered to the phone reading the page.

download-android = Androidアプリを入手

## The wiki's maps.

map-none = 海外wikiでこのマップの地図が掲載されていません。
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
gate-dead-end = 反対側の孤立エリアからのみ
gate-isolated = 反対側の孤立エリアにて通行

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
