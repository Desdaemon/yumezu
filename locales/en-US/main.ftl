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
dump-failed = The world dump could not be loaded.

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
layered-hint = Separate worlds into layers of depths
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
world-author-hint = Show every world by this author
world-map-hint = Show the wiki's map of this world
world-move-up = Show parent map
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

## The route home.

route-length = { $count ->
        [one] 1 connection from the origin
       *[other] { $count } connections from the origin
    }
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
menu-open-wiki = Open on yume.wiki

## The rocker in the corner.

rocker-shallower = Shallower
rocker-deeper = Deeper

## The settings tab.

hub-push = hub push
hub-push-hint = The higher the value, the harder bigger worlds' repulsion force is
ui-scale = UI scale
ui-scale-hint = How large the panel and its text are drawn
show-controls = Show controls
github-link = yumezu on github
android-link = Download for Android

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

## The Android app, offered to the phone reading the page.

download-android = Get the Android app

## The wiki's maps.

map-none = The wiki draws no map of this world.
map-missing = Map image not available.
map-fit = Fit the whole map in the window
map-maximize = Fill the screen with the window
map-restore = Put the window back where it was

## What a connection asks of a player walking it.
##
## The bare name of the condition, for the connections the wiki writes no words of its own about.

gate-effect = needs an effect
gate-chance = by chance
gate-seasonal = seasonal
gate-locked = unlocked from opposite entrance
gate-locked-condition = locked, conditional
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
