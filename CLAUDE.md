Performance

Performance is this project's first priority, ahead of everything else in this
document. Helix is judged on CPU and RAM: idle CPU near zero, no UI-thread
stalls, bounded and predictable memory, small binary. When a clean design and a
fast one disagree, take the fast one and make it readable.

Non-negotiables:

Nothing that blocks may run on the UI thread. File IO, git2 calls, subprocesses
and anything that can take milliseconds go to cx.background_executor(), with the
result applied back through this.update.

render() is read-only and allocation-light. No syscalls, no canonicalize, no
read_dir, no git2, no config parsing, no deep clones of snapshots or row lists.
Precompute when the data changes, not when it is drawn.

Nothing may notify or animate on a timer without a reason. Every cx.notify()
rebuilds the whole window's element tree, so a recurring notifier multiplies the
cost of every render path. Notify only when what is drawn actually changed, and
coalesce bursts instead of forwarding them one by one.

Memory is bounded on purpose. Caches get an eviction policy, scrollback gets a
cap, long-lived buffers get reused rather than reallocated per frame.

Long lists are virtualized. Anything that can grow past a screenful uses
uniform_list rather than building every row.

Before adding a dependency or an abstraction, check what it costs at runtime and
in binary size. Measure before and after; a change that cannot be measured is not
a performance change.

Crate Boundaries

Every crate under src/ owns one subject and nothing else. A crate never absorbs
work that belongs to another just because that was the shorter path from where
the code happened to be written.

helix-ui draws and nothing more. It maps domain values to colours, icons,
labels and layout, and it routes input. It never decides. Whether a pull request
can be merged, whether a branch is eligible for review, how a rollup of checks
collapses into one verdict, how a listing folds into a per-branch map: all of
that is domain logic and lives in the crate that owns the subject.

The test is whether the rule survives without a window. If a decision would
still be true in a headless run, it belongs in a domain crate, behind a name,
with tests. If the answer changes when the theme or the layout changes, it
belongs in helix-ui.

Domain crates do not reach the other way either. helix-git, helix-github,
helix-terminal, helix-worktree and helix-state depend on gpui for nothing. They
return plain values and let the UI decide how to draw them.

A helper placed in the wrong crate is a defect even when it compiles and the
tests pass. Move it rather than duplicating it.

Components

Reach for gpui-component before building a control by hand. A div with its own
padding, rounding, cursor and hover states is almost always a Button; a key cap
is a Kbd; a filter over a list is a List with a delegate; a dropdown is
dropdown_menu. Hand-rolled controls drift apart, miss the platform bindings and
carry styling that has to be maintained twice.

Read what the component owns before adopting it. The library is written around
components that keep their own state, and where that state is something this app
must react to or control, the component is the wrong tool no matter how well its
API reads:

The panel group models panels as proportions of a container and pushes leftover
drag onto its neighbours, so fixed sidebars with a flexible centre are not
expressible in it. Tree owns expansion and emits nothing, which leaves no hook
to scan a directory when it opens, so a lazy tree cannot use it. Tab overwrites
height, text size, background, border and radius with its variant's own, so a
tab that is not a full-radius pill cannot use it.

Three checks answer this before any code is written. Does the app stay the owner
of the state it needs? Is every visual the component decides one this project is
happy to inherit? Is the behaviour it replaces still reachable, keyboard included?
A no to any of them means leave the hand-written version alone and say why.

Anything drawn by a component takes its colours from the component theme, so a
token it reads and sync_component_theme does not set arrives in the library's own
palette. Adding a component means checking which tokens it reads and mapping them
to ours in the same change, never picking a colour by eye.

Code Style
Rust

Write clean, idiomatic, readable, and maintainable Rust code.

Indentation

Use 2 spaces for indentation.

Never use tabs for indentation.
Use exactly 2 spaces for each indentation level.
Keep indentation consistent throughout the entire codebase.
Do not mix tabs and spaces.

Example:

fn process_user(user: &User) -> Result<(), Error> {
  if user.is_valid() {
    let data = load_user_data(user)?;

    save_data(data)?;
  }

  Ok(())
}

Vertical Spacing

Use blank lines intentionally to improve readability.

Separate different functions with a blank line.
Separate different struct, enum, trait, and impl declarations with a blank line.
Inside functions, use blank lines to separate distinct logical steps or blocks of logic.
Keep statements that belong to the same logical operation together.
Do not add a blank line between every statement.
Never use multiple consecutive blank lines.
Avoid both overly dense code and excessive vertical spacing.
Give the code enough visual breathing room to make logical sections easy to scan.
Blank lines should separate ideas, not individual statements.

Example:

fn process_user(user: &User) -> Result<(), Error> {
  let data = load_user_data(user)?;
  let validated_data = validate_data(data)?;

  save_data(validated_data)?;

  Ok(())
}

fn validate_data(data: Data) -> Result<Data, Error> {
  if data.is_valid() {
    Ok(data)
  } else {
    Err(Error::InvalidData)
  }
}

Concrete rules

Apply these exactly. Dense code with no blank lines is the single most common defect.

Put a blank line:

Before and after every multi-line block (if, match, for, while, loop, closure body).
After a group of guard clauses, before the work the guards were protecting.
Before the final return value or trailing expression of a function, when the function has more than one step.
Between setup and the operation that consumes it (a builder and the call that uses it).
Between distinct logical steps inside a loop or closure body.

Do not put a blank line:

Between consecutive short guard clauses that read as one validation group.
Between statements that form a single operation (a binding and the call that mutates it).
Inside a method chain. A fluent chain is one expression: never split it with a blank line.
After an opening brace or before a closing brace.
Twice in a row. Never two consecutive blank lines.

Example:

fn collect_ahead_behind(repo: &Repository, snap: &mut GitSnapshot) {
  let Ok(head) = repo.head() else { return };

  if !head.is_branch() {
    return;
  }

  let Ok(name) = head.shorthand() else { return };
  let Ok(branch) = repo.find_branch(name, BranchType::Local) else {
    return;
  };

  snap.upstream = branch.upstream().ok().and_then(|up| up.name_string());

  if let Ok((ahead, behind)) = repo.graph_ahead_behind(local, remote) {
    snap.ahead = ahead;
    snap.behind = behind;
  }
}

Bad, because the guards, the work and the result are glued into one wall:

fn collect(repo: &Repository) -> Result<Snapshot> {
  let head = repo.head()?;
  if !head.is_branch() {
    return Err(Error::Detached);
  }
  let mut snap = Snapshot::default();
  for oid in walk.take(15).flatten() {
    let commit = repo.find_commit(oid)?;
    let summary = commit.summary().unwrap_or_default();
    if summary.is_empty() {
      continue;
    }
    snap.commits.push(summary.to_string());
  }
  Ok(snap)
}

Comments

Use only necessary comments.

Do not add comments to code that is already self-explanatory from its naming, structure, or implementation.

Prefer improving the code itself over adding a comment to explain confusing code.

Comments are appropriate for:

Non-obvious implementation decisions.
Complex business rules.
Unexpected behavior.
Limitations or workarounds.
Important invariants or assumptions.
Reasons behind unusual implementation choices.

Avoid comments that simply describe what the next line does.

Bad:

// Increment counter
counter += 1;


Good:

// The external API may return duplicate events,
// so already-processed event IDs are ignored.
if processed_ids.contains(&event_id) {
  return Ok(());
}

General Rust Principles
Follow idiomatic Rust conventions.
Use 2 spaces for indentation.
Never use tabs for indentation.
Prefer simple solutions over unnecessary abstractions.
Use clear and descriptive names.
Keep functions focused on a single responsibility.
Avoid unnecessary duplication.
Remove unused imports, variables, and abstractions.
Prefer Result and Option for explicit error and absence handling.
Prefer ? for error propagation.
Avoid unnecessary unwrap() and expect().
Avoid unsafe unless genuinely necessary.
Do not over-engineer simple problems.
Prefer readable code over clever code.
Keep related logic together.
Separate unrelated logic into clear sections.
Formatting

Use consistent formatting throughout the project.

Use 2 spaces per indentation level.

Do not use tabs.

Use blank lines to improve the visual structure of the code while keeping related statements together.

Do not add excessive whitespace just to make the code longer.

Do not use multiple consecutive blank lines.

Code Organization

Organize code logically and consistently.

Prefer the following general structure when appropriate:

Imports
Constants
Types
Structs and enums
Traits
Implementations
Helper functions
Public functions or entry points

Do not force this structure when the project architecture calls for a different organization.

Error Handling

Use Rust's error handling mechanisms idiomatically.

Prefer Result for operations that can fail.
Prefer Option when a value may legitimately be absent.
Use ? for error propagation.
Avoid unwrap() unless failure is genuinely impossible or explicitly intentional.
Avoid expect() unless the invariant being asserted is clear and justified.
Do not silently ignore errors.
Maintainability

Write code that is easy for another developer to understand and modify.

Before finalizing code, check:

Is the code unnecessarily complex?
Are there unnecessary abstractions?
Are there duplicated sections?
Are variable and function names clear?
Are imports and variables actually needed?
Is error handling appropriate?
Is the indentation consistently 2 spaces?
Is the code properly spaced vertically?
Are there unnecessary comments?
Can the code be simplified without losing clarity?
Core Principle

Write code as an experienced Rust developer would:

Clean, idiomatic, consistently indented with 2 spaces, well-spaced, easy to scan, and minimally commented.

The code itself should communicate what it does.

Comments should explain why, not simply what.
