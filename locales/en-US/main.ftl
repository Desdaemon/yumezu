# Everything this app says, in English.
#
# One message per thing said, named for what is said rather than for where it is said: a hint and
# a label that happen to read alike are still two messages, because the next language is not
# obliged to agree that they read alike. See `src/i18n.rs`, which is what reads this.
#
# English is also the fallback: a message left out of another language is read from here instead,
# so nothing here may be deleted while another file still leans on it.

# What this language calls itself, which is what the picker offers it as. Every language names
# itself, so the picker reads the same whichever one is being spoken.
language-name = English
language = Language

## The frame before the graph: what is said while the world dump is on its way in, and what is
## said if it never arrives. See `world::load`.

dump-loading = Loading worlds…
dump-failed = The worlds could not be loaded.

# What the server says it is doing, for a run that arrived while it was still building its first
# dump and has to wait it out. Named for the stage rather than for the server's own word for it:
# see `world::building`, which is what maps the one onto the other.
dump-task-changes = Querying for wiki updates...
dump-task-worlds = Loading worlds...
dump-task-connections = Loading connections...
dump-task-assembling = Finalizing the graph...

## The sidebar and its tabs.

tab-worlds = Worlds
tab-authors = Authors
tab-versions = Versions
hide-sidebar = Hide the sidebar
show-sidebar = Show the sidebar

## The graph tab: what is on screen, and how it is laid out.

fps = { $fps } fps
graph-size = { $worlds } worlds, { $connections } connections
dimensions-2d = 2D
dimensions-3d = 3D
layered = layered
layered-hint = Separates worlds into layers by depth
search-worlds = Search worlds
search-authors = Search authors
search-versions = Search versions

# How many worlds, said so that one of them does not read as a bug.
worlds = { $count ->
        [one] 1 world
       *[other] { $count } worlds
    }

# How much of a list is being shown. The plain form is for a list nothing has been typed into;
# the cut form is what a search narrows it to.
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

## Where there is still somewhere to go, offered with nothing selected while a frontier is drawn.

untaken-worlds = Get hints
untaken-worlds-hint = Revisit these worlds that connect to places you haven't been.

## The route home.

route-length = { $count ->
        [one] 1 connection from the origin
       *[other] { $count } connections from the origin
    }
# Directions between two worlds, which start somewhere other than the origin.
path-length = { $count ->
        [one] 1 connection from { $origin }
       *[other] { $count } connections from { $origin }
    }
no-path = No way there from here.
# The ways a set of directions can be walked, by how much of the game the player already has.
way-length = { $count ->
        [one] 1 connection
       *[other] { $count } connections
    }
# The window offering the ways between two worlds, which only opens where there are several.
directions-title = { $origin } → { $destination }
way-via = via { $world }
zoom-in-world = Zoom in on world
zoom-out-route = Zoom out to route
trace-route = Trace the route to this world

## Ways on from a world.

no-forward-connections = No forward connections.
forward-connections = { $count ->
        [one] 1 forward connection
       *[other] { $count } forward connections
    }

## What hangs off a world.

no-notable-descendants = No notable descendants.
notable-descendants = Notable descendants:
# One of those, with what makes it worth naming: whether it is a junction or a dead end, and how
# many worlds it touches.
notable-world = { $title }  ({ $kind }, { $degree })

## The catalogs.

author-row = { $name }  ({ $worlds })
# A release, and what it brought. The dated form is the usual one; the wiki does not date a
# handful of releases, and those are named without a date rather than with an empty one.
version-row = { $name }  ({ $worlds })
version-row-dated = { $name }  ({ $worlds }, { $released })
version-released = released { $released }
version-added = { $worlds } added
layer-depth = Depth { $depth }

## The menu a right-click opens.

menu-descendants = Highlight descendants
# Offered only while another world is lit, which is the far end of the directions.
menu-directions-to = Directions to here
menu-directions-to-hint = Show the way here from { $world }.
menu-directions-from = Directions from here
menu-directions-from-hint = Show the way from here to { $world }.
menu-open-wiki = Open on yume.wiki

## The rocker in the corner.

rocker-shallower = Shallower
rocker-deeper = Deeper

## The settings tab.

hub-push = hub push
hub-push-hint = Higher values push bigger worlds further from their neighbours
link-reach = link reach
link-reach-hint = How far apart two connected worlds may settle, in layers. One-way connections are not held to it
ui-scale = UI scale
ui-scale-hint = How large the panel and its text are
antialias = Smooth edges
antialias-hint = Softens the edges of the graph. Turn this off first when the frame rate falls
    short of your display.
antialias-restart = Takes effect the next time yumezu starts.

leaning = lean onto pointed worlds
leaning-hint = Pointing at a world in a list carries the view onto it, slowly. Turn it off to hold the view still.
show-controls = Show controls
clear-cache = Clear downloads
clear-cache-hint = Deletes the world pictures kept between runs. The next look at a world fetches it again
clear-cache-clearing = Clearing...
clear-cache-done = Downloads cleared
clear-cache-failed = The downloads could not be cleared
update-check = Check for updates
update-check-hint = Checks GitHub for a release newer than this build
update-checking = Checking...
update-current = This is the newest release
update-ready = { $version } is available
update-install = Install it
update-installing = Downloading...
update-installed = Installed. It takes effect the next time yumezu starts.
update-failed = The update could not be fetched
# A date and a time, in the reader's own zone. The parts arrive separately, each already padded to
# the width it is always written at, because the order and the separators are the language's.
stamp = { $year }-{ $month }-{ $day } { $hour }:{ $minute }

last-update = Updated { $when }
last-update-hint = When this copy of the wiki data was built
last-full-update = Last reset { $when }
last-full-update-hint = Full resets read fresh data from the wiki and remove gaps left by renamed worlds.

## The player's own game: signing in to YNOproject, and drawing only what that account has seen.

yno = Exploration progress
yno-loading = Loading visited worlds...
yno-hint = Sign in to YNOproject to track your exploration progress.
yno-user = Username
yno-password = Password
yno-sign-in = Sign in
yno-sign-out = Sign out
yno-working = Signing in...
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
frontier = Frontier Mode
frontier-hint = Shows only worlds you've visited. Worlds one step beyond them appear as unvisited locations.
unvisited-location = Unvisited Location
github-link = yumezu on github
download-for = Download for {$platform}

## The controls, named on the first run.

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

## The app, offered to a page whose browser has a package to install.

download-app = Get the {$platform} app

## The wiki's maps.

map-none = The wiki draws no map of this world.
map-missing = Map image not available.
map-fit = Fit the whole map in the window
map-maximize = Fill the screen with the window
map-restore = Put the window back where it was

## What a connection demands of a player walking it.
##
## The bare name of the condition, for the connections the wiki writes no words of its own about.

gate-effect = needs an effect
gate-chance = by chance
gate-seasonal = seasonal
gate-locked = unlocked from opposite entrance
gate-locked-condition = locked, conditional
gate-exit-point = back out through a shortcut
gate-dead-end = only from isolated section
gate-isolated = leads to isolated section

# And the same conditions where the wiki does write words. The effects are listed as the wiki
# lists them, comma separated, rather than joined into a sentence: the wiki does not say whether
# one of them is enough or all of them are needed, and an "and" or an "or" here would be this app
# saying which.
gate-effect-detail = needs { $effects }
gate-chance-detail = { $chance } chance
gate-seasonal-detail = { $season ->
        [Spring] in Spring
        [Summer] in Summer
        [Fall] in Fall
        [Winter] in Winter
       *[other] in { $season }
    }

## The game's thirty-five effects, in the order it gives them. The wiki writes a condition in
## English alone, so these are what it already says; they are messages all the same, because a
## language that names them differently has nowhere else to say so.

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

## Which ways round a connection can be walked, in a sentence.
##
## The two directions are named apart, because a connection can be free one way and locked the
## other, and a reader deciding whether to walk it needs the way they are about to walk.

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
