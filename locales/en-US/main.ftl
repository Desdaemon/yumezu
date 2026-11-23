# English UI strings, loaded by `src/i18n.rs`.
#
# Message IDs name the meaning, not the location: a label and a hint with the same English text are
# still two messages, since another language may word them differently.
#
# English is the fallback for messages missing from other languages, so do not delete a message
# another locale still relies on.

# The language's name in itself (endonym), shown in the language picker.
language-name = English
language = Language

# Window theme picker: the default theme, or one of the game's skins.
system-theme = Theme
system-theme-painted = Default

## Loading screen, while the world data downloads. See `world::load`.

dump-loading = Loading worlds…
dump-failed = Could not load worlds.
# The browser refused WebGL2, usually until it restarts after its graphics process crashed.
webgl-refused = Your browser has turned off 3D graphics for this page. Quit the browser completely, open it again, and reload.

# Server progress while it builds its first data dump. IDs name the stage; `world::building` maps
# the server's status to these.
dump-task-changes = Querying for wiki updates…
dump-task-worlds = Loading worlds…
dump-task-connections = Loading connections…
dump-task-assembling = Finalizing the graph…

## The browse window and its tabs.

tab-worlds = Worlds
tab-authors = Authors
tab-versions = Versions

## The graph tab: statistics and layout options.

fps = { $fps } fps
graph-size = { $worlds } worlds, { $connections } connections
dimensions-2d = 2D
dimensions-3d = 3D
arrangement = Arrangement
arrangement-flat = Flat
arrangement-radial = Radial
arrangement-free = Free
arrangement-tree = Tree
arrangement-orbital = Orbital
search-worlds = Search worlds
search-authors = Search authors
search-versions = Search versions

# World count, with a singular form.
worlds = { $count ->
        [one] 1 world
       *[other] { $count } worlds
    }

# List counts. The plain form is the unfiltered list; the `-cut` form is a search result.
showing-authors = { $total ->
        [one] 1 author
       *[other] { $total } authors
    }
showing-authors-cut = { $shown } of { $total } authors
showing-versions = { $total ->
        [one] 1 version
       *[other] { $total } versions
    }
showing-versions-cut = { $shown } of { $total } versions

## The selected world.

world-author = by
world-author-hint = Show author's worlds
world-english-name = English name
world-english-wiki = Open the English wiki
world-map-hint = View maps
world-move-up = Show parent world
world-connections = { $count ->
        [one] 1 connection,
       *[other] { $count } connections,
    }
world-descendants = { $count ->
        [one] 1 descendant
       *[other] { $count } descendants
    }
dead-end = dead end
junction = junction

nothing-selected =
    Click a world to trace its route to the origin, or right-click it for more.

## Exploration hints, shown with nothing selected in frontier mode.

untaken-worlds = Get hints
untaken-worlds-hint = Revisit these worlds that connect to places you haven't been.

## The route to the origin world.

route-length = { $count ->
        [one] 1 connection from the origin
       *[other] { $count } connections from the origin
    }
# Directions between two worlds, starting somewhere other than the origin.
path-length = { $count ->
        [one] 1 connection from { $origin }
       *[other] { $count } connections from { $origin }
    }
no-path = No path between these worlds.
# Length of one alternative path in the directions window.
way-length = { $count ->
        [one] 1 connection
       *[other] { $count } connections
    }
# Directions window title; the window opens only when there are several paths.
directions-title = { $origin } → { $destination }
way-via = via { $world }
zoom-in-world = Zoom in on world
zoom-out-route = Zoom out to route
trace-route = Trace the route to this world

## Forward connections of a world.

no-forward-connections = No forward connections.
forward-connections = { $count ->
        [one] 1 forward connection
       *[other] { $count } forward connections
    }

## Descendants of a world.

no-notable-descendants = No notable descendants.
notable-descendants = Notable descendants:
# A notable descendant: `$kind` is junction or dead end, `$degree` its connection count.
notable-world = { $title }  ({ $kind }, { $degree })

## The catalogs.

author-row = { $name }  ({ $worlds })
# A game version and the worlds it added. A few versions have no release date on the wiki and use
# the undated form.
version-row = { $name }  ({ $worlds })
version-row-dated = { $name }  ({ $worlds }, { $released })
version-released = released { $released }
version-added = { $worlds } added
layer-depth = Depth { $depth }

## Context menu (right click).

menu-descendants = Highlight descendants
# Shown only while another world is selected; that world is the other end of the directions.
menu-directions-to = Directions to here
menu-directions-to-hint = Show the way here from { $world }.
menu-directions-from = Directions from here
menu-directions-from-hint = Show the way from here to { $world }.
menu-open-wiki = Open on yume.wiki

## The depth stepper ("rocker") in the corner.

rocker-shallower = Shallower
rocker-deeper = Deeper
reset-camera = Reset camera

## The settings tab.

hub-push = Hub repulsion
hub-push-hint = How strongly larger worlds repel their neighbors
link-reach = Max connection length
link-reach-hint = How far apart connected worlds settle, in layers. Each arrangement keeps its own. One-way connections are exempt
ui-scale = UI scale
antialias = Anti-aliasing
antialias-hint = Smooths jagged edges. Turn off first if the frame rate drops below your display's
    refresh rate.
antialias-restart = Restart yumezu to apply.

leaning = Follow worlds hovered in lists
leaning-hint = Hovering a world in a list slowly centers the view on it.
show-controls = Show controls
clear-cache = Clear cache
clear-cache-hint = Deletes cached world images. They are downloaded again when needed
clear-cache-clearing = Clearing…
clear-cache-done = Cache cleared
clear-cache-failed = Could not clear the cache
update-check = Check for updates
update-check-hint = Checks GitHub for a release newer than this build
update-checking = Checking…
update-current = Up to date
update-ready = { $version } is available
update-install = Install update
update-installing = Downloading…
update-installed = Installed. Restart yumezu to apply.
update-failed = Update failed
# Local date and time. The parts arrive zero-padded and separate, so each language sets the order
# and separators.
stamp = { $year }-{ $month }-{ $day } { $hour }:{ $minute }

last-update = Updated { $when }
last-update-hint = When this copy of the wiki data was built
last-full-update = Last reset { $when }
last-full-update-hint = Full resets read fresh data from the wiki and remove gaps left by renamed worlds.

## YNOproject sign-in and exploration progress.

yno = Exploration progress
yno-loading = Loading visited worlds…
yno-hint = Sign in to YNOproject to track your exploration progress.
yno-user = Username
yno-password = Password
yno-sign-in = Sign in
yno-sign-out = Sign out
yno-working = Signing in…
yno-signed-out = That session has expired. Sign in again.
yno-signed-in = Signed in.
yno-completion = { $seen } / { $worlds } ({ $percent }%)
yno-completion-hint = How many of the worlds you have discovered.
yno-refresh = Refresh
yno-refresh-hint = Reveals new worlds you've since visited.
yno-promise = Your username and password are used only to read your exploration progress from YNOproject. yumezu never shares them and never changes your account.
yno-source = Audit the YNOproject connection
menu-reveal = [Debug] Reveal
menu-reveal-hint = Mimics adding a new world from the Refresh button.
frontier = Frontier mode
frontier-hint = Shows only worlds you've visited. Worlds one step beyond them appear as unvisited locations.
unvisited-location = Unvisited location
github-link = yumezu on GitHub
download-for = Download for {$platform}

## Controls guide, shown on first run.

guide-title = Controls
guide-inputs = Inputs
guide-fly-input = W/S
guide-fly-action = Fly forward/backward
guide-strafe-input = A/D
guide-strafe-action = Strafe
guide-orbit-mouse-input = Left mouse
guide-orbit-mouse-action = Orbit
guide-orbit-touch-input = One finger
guide-orbit-touch-action = Orbit
guide-options-input = Right mouse
guide-options-action = Options
guide-pan-input = Right mouse (hold)
guide-pan-action = Pan
guide-pinch-input = Two fingers
guide-pinch-action = Zoom/Pan
guide-scroll-input = Scroll wheel
guide-scroll-action = Zoom
guide-rocker = The rocker
guide-rocker-body =
    The two arrows in the bottom-right corner select an entire layer of the graph at once.
guide-got-it = Got it
dont-show-again = Don't show this again

## Web build: link to the native app for the visitor's platform.

download-app = Get the {$platform} app

## The wiki's maps.

map-none = The wiki has no map of this world.
map-missing = Map image not available.
map-fit = Fit map to window
map-maximize = Maximize
map-restore = Restore

## Connection requirements. The bare form is for connections with no wiki detail text.

gate-effect = needs an effect
gate-chance = by chance
gate-seasonal = seasonal
gate-locked = unlocked from opposite entrance
gate-locked-condition = locked, conditional
gate-exit-point = back out through a shortcut
gate-dead-end = only from isolated section
gate-isolated = leads to isolated section

# The same requirements with wiki detail. Effects stay a comma-separated list: the wiki does not
# say whether any or all are needed, so do not join them with "and" or "or".
gate-effect-detail = needs { $effects }
gate-chance-detail = { $chance } chance
gate-seasonal-detail = { $season ->
        [Spring] in Spring
        [Summer] in Summer
        [Fall] in Fall
        [Winter] in Winter
       *[other] in { $season }
    }

## The game's 35 effects, in game order. The English values match the wiki's names; they are
## messages so other languages can translate them.

effect-separator = {", "}
effect-bike = Bike
effect-boy = Boy
effect-chainsaw = Chainsaw
effect-lantern = Lantern
effect-fairy = Fairy
effect-spacesuit = Spacesuit
effect-glasses = Glasses
effect-rainbow = Rainbow
effect-wolf = Wolf
effect-eyeball-bomb = Eyeball Bomb
effect-telephone = Telephone
effect-maiko = Maiko
effect-twintails = Twintails
effect-penguin = Penguin
effect-insect = Insect
effect-spring = Spring
effect-invisible = Invisible
effect-gakuran = Gakuran
effect-plaster-cast = Plaster Cast
effect-stretch = Stretch
effect-haniwa = Haniwa
effect-trombone = Trombone
effect-cake = Cake
effect-child = Child
effect-red-riding-hood = Red Riding Hood
effect-tissue = Tissue
effect-bat = Bat
effect-polygon = Polygon
effect-teru-teru-bozu = Teru Teru Bōzu
effect-marginal = Marginal
effect-drum = Drum
effect-grave = Grave
effect-crossing = Crossing
effect-bunny-ears = Bunny Ears
effect-dice = Dice

## Connection direction. The two directions are worded separately, since a connection can be
## open one way and locked the other.

walk-freely = freely
walk-free-both = No restrictions.
walk-one-way = One-way.
walk-no-entry = No entry from here.
walk-none = Currently inaccessible.
walk-dead-end = Connected via isolated section only.
walk-isolated = Connects to isolated section.
walk-locked-out = Unlockable from opposite entrance.
walk-locked-back = Unlocks access to this area from opposite entrance.
walk-both =
    From here: { $out }
    To here: { $back }.
walk-out-only = From here only: { $out }
walk-back-only = To here only: { $back }
