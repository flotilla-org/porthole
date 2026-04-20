# Window Evidence Experience Report

Date: 2026-04-20

Audience: me, and agents I will work with later

## Context

This note records the experience of trying to gather clean visual evidence for a
Ghostty bug report around kitty graphics geometry deletes.

The concrete task was:

- build fresh upstream Ghostty `main`
- run a small visual repro in Ghostty and Kitty
- capture before/after screenshots for both terminals
- package the screenshots alongside a minimal repro and draft issue

This sounds straightforward. In practice, a lot of the effort went into
controlling windows, identifying the correct window, keeping it alive long
enough to capture, and getting deterministic screenshots out of desktop apps.

The session was heavily biased toward:

- macOS
- Ghostty
- Kitty
- agent-driven shell commands
- a workflow where the agent needs evidence, not just text output

So this is not a general desktop automation survey. It is a report from a real
session, with real friction, that suggests a class of tooling I want.

## What Actually Happened

At a high level, the path looked like this:

1. Build fresh Ghostty from a clean clone.
2. Discover a local toolchain issue unrelated to the actual bug.
3. Fix the toolchain so the app could be built.
4. Launch the freshly built Ghostty app.
5. Try to run the repro inside it.
6. Try to capture the correct window.
7. Realize that multiple Ghostty windows and processes existed, including old
   ones and different app bundles.
8. Add window enumeration and PID/window-ID discovery just to know which thing
   to screenshot.
9. Try to automate `Enter` presses through accessibility APIs.
10. Discover that app launch style, shell lifetime, and window naming all
    affected whether the automation remained attached to the correct thing.
11. Build a standalone repro to reduce surrounding context.
12. Discover a repro bug caused by extracting too much and dropping a critical
    transport detail.
13. Repair the repro, then repeat the screenshot loop.
14. Discover that different launch methods behaved differently across Ghostty
    and Kitty.
15. End up using different automation strategies for different terminals.

This was all in service of obtaining eight screenshots and a small repro script.

That is the important lesson: the actual bug investigation was not the hard
part. The hard part was desktop orchestration and evidence collection.

## Friction Points

### 1. Starting the right app was annoyingly indirect

Launching an app on macOS through `open -na ... --args ...` works, but it is
not a great interface for agentic use.

The real requirements were:

- start this app bundle
- run this command in the window
- keep the window alive or do not keep it alive, depending on phase
- distinguish this launched instance from others already on the desktop

Instead, we had to piece this together out of:

- `open`
- shell quoting
- temporary shell scripts
- title matching
- process enumeration

This is workable, but brittle and expensive in agent attention.

### 2. Window identity was too weak and too implicit

The central repeated pain was not “how do I capture a screenshot”, but “which
window is actually mine”.

We had to infer identity from combinations of:

- process id
- window title
- app name
- app bundle path
- current command running in the terminal

This broke down in several ways:

- multiple Ghostty windows existed simultaneously
- different Ghostty app bundles were running
- windows changed title over time
- shell exit changed visible state
- the process used to launch the window was not always the process that owned
  the long-lived UI

What I wanted was a stable, tool-level concept of:

- launch handle
- window handle
- capture handle

Once a window is launched for a task, the agent should not be back to scraping
global window state and guessing.

### 3. Evidence capture depended too much on timing

The before/after screenshots were sensitive to:

- whether the app had finished drawing
- whether the repro had reached the pause point
- whether the `Enter` press had been processed yet
- whether the window had already exited
- whether the post-delete state had been painted

We ended up using a mix of:

- fixed sleeps
- manual intervention
- accessibility-driven key injection
- “linger for N seconds” behavior in the repro

This is the wrong abstraction level. The agent should be able to say things
more like:

- capture when this window is stable
- capture before sending input
- send Enter
- capture after the frame changes

Right now we are doing crude temporal approximation.

### 4. Accessibility automation is useful, but not enough

Once accessibility was enabled, sending `Enter` became possible through
`osascript` and System Events. That helped.

But it is still an awkward fit because:

- it depends on global UI permissions
- it targets frontmost processes rather than task-specific handles
- it introduces another separate automation channel
- it is not coupled to window identity or evidence collection

Accessibility input is one necessary capability, but it is not the tool I
actually want. It is just a partial escape hatch.

### 5. Window capture is not the same as task capture

We repeatedly hit the gap between:

- “a screenshot API exists”

and

- “I can robustly capture the window that matters for this task”

Even after finding the right window, capture could fail or become stale because:

- the window id changed
- the process exited
- the app switched state
- the capture target no longer matched the task

The missing abstraction is not image capture itself. It is task-scoped visual
capture.

### 6. Repro extraction was valuable, but also risky

Extracting a minimal repro was the right call for the bug report. It improved
clarity.

But the extraction also created a new failure mode: I accidentally removed the
base64 encoding behavior that the larger smoke harness relied on, so the new
repro initially drew no images.

This is a reminder that evidence workflows tend to involve:

- a test artifact
- an app launcher
- a capture layer
- a packaging layer

If those are all ad hoc, an agent has too many places to make a subtle mistake.

### 7. The evidence packaging step itself was fragmented

By the end, we had:

- a report folder
- a repro script
- a draft issue
- code notes
- references
- screenshots

That part was fine.

What was not fine was that there was no integrated notion of a “capture
session” tying these together. The agent had to manually assemble:

- what was run
- in which app
- on which build
- against which window
- with which screenshots

This should probably be first-class.

## What Would Have Helped Immediately

These are the capabilities that would have materially shortened this session.

### Stable launch and window handles

Something like:

```sh
porthole launch --app /tmp/ghostty-main-smoke/zig-out/Ghostty.app \
  --title ghostty-repro-cell \
  --cmd 'python3 repro.py cell'
```

and then:

```sh
porthole window show <launch-id>
porthole screenshot <launch-id> --output before.png
```

The critical thing is that the agent should not have to rediscover the window
through global enumeration after launching it.

### Built-in “keep alive” and lifecycle control

We repeatedly had to decide whether a terminal should:

- exit when the command finishes
- stay alive in a shell
- stay alive for N seconds
- stay alive until explicitly closed

This should be a normal launch option, not something reconstructed via shell
scripts and `exec zsh -i`.

### Integrated input targeting

Instead of separate UI scripting:

```sh
porthole key <window-id> Enter
porthole text <window-id> '...'
```

This should target the specific managed window, not “whatever is frontmost”.

### Better capture semantics

I want capture primitives closer to:

```sh
porthole screenshot <window-id> --output before.png
porthole key <window-id> Enter
porthole screenshot <window-id> --after-paint --output after.png
```

Potentially also:

```sh
porthole record <window-id> --seconds 5 --output repro.mov
```

### Task/session grouping

Something like:

```sh
porthole session start ghostty-delete-bug
porthole session attach-artifact ghostty-delete-bug before.png
porthole session attach-artifact ghostty-delete-bug after.png
porthole session metadata ghostty-delete-bug set terminal=ghostty
```

This would reduce the “manual evidence bundle assembly” problem.

### Present-to-user support

The agent sometimes needs not only to capture a window, but to deliberately
show it to the human:

- bring to front
- place on a particular monitor
- highlight it
- leave it visible while discussion happens

This feels adjacent to evidence capture and should likely be in the same tool.

## Implications for Porthole

The shape that emerges from this session is not “desktop automation in general”.
It is narrower and more useful:

- launch apps in a way that is trackable
- obtain stable handles for windows owned by that launch
- drive those windows with targeted input
- capture screenshots and short recordings
- preserve enough metadata that the outputs make sense later
- optionally present those windows to the user

The most important idea is that `porthole` should probably be task-oriented,
not just a bag of window commands.

The agent does not really want:

- enumerate every window on the desktop

It wants:

- launch this thing for this task
- know which window is the one I care about
- drive it
- capture it
- package the evidence

If `porthole` degenerates into thin wrappers over OS-level window APIs, it will
be less useful than it sounds. The real value is in carrying task identity
through launch, control, and capture.

## Constraints and Realities

Some caution points from this session:

- behavior will differ significantly by platform
- macOS app launch semantics are awkward enough that they should be treated as
  a first-class design concern
- terminal apps are a very useful initial target because they expose both
  graphical windows and text commands
- accessibility permissions matter, but they should not be the whole story
- window ids and titles are not stable enough to be the primary abstraction

Also: building cross-platform too early would be a trap. The abstraction should
probably be designed with multiple platforms in mind, but validated in one real
environment first.

For me, that likely means:

- start with macOS
- focus on terminal and developer-app workflows
- optimize for evidence collection and presentation

## Open Questions

- Should `porthole` manage launches itself, or attach to already-running apps as
  a secondary mode?
- Should screenshot and recording outputs automatically live inside a
  task/session directory?
- How much of input should be semantic (`key Enter`) versus low-level
  (`raw event stream`)?
- Should “bring to front” and “show this to the user” be explicit API concepts?
- Is it worth modeling monitor placement and window geometry early, or is that a
  distraction from the core evidence loop?
- How much can be done without accessibility privileges, and what should the
  degraded mode look like?
- Should `porthole` return structured metadata about each capture by default
  (timestamp, app, window title, bundle path, dimensions, etc.)?

## Bottom Line

The main difficulty in this session was not reproducing a terminal bug. It was
reliably launching, identifying, driving, and capturing desktop windows as part
of an agent workflow.

That is a strong enough pain signal to justify a focused companion tool.

If I build `porthole`, the first success criterion should be simple:

> It should make the exact workflow from this session feel boring.

