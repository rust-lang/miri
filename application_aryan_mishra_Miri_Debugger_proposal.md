### About

1. **Full Name:** Aryan Mishra
2. **Contact info (public email):** aryanmi2001@gmail.com
3. **Discord handle:** alaotach
4. **GitHub profile link:** https://github.com/alaotach
5. **Twitter, LinkedIn:** [Twitter](https://x.com/alaotach) [LinkedIn](https://linkedin.com/in/alaotach)
6. **Time zone:** IST (UTC+05:30)
7. **Link to a resume:** [Resume](https://drive.google.com/file/d/1hnyt2KPOfiS1uNWjsXIU33ENeKOni_Yh/view?usp=sharing)

---

### University Info

1. **University name:** Jawaharlal Nehru University
2. **Program:** B.Tech (Electronics and Communication Engineering)
3. **Year:** Second Year
4. **Expected graduation date:** 2028

---

## Motivation and Past Experience

### 1. Have you worked on or contributed to a FOSS project before?

Yes. My most relevant prior work is two production-quality Rust TUI applications I built entirely on my own, both of which directly demonstrate the skills this project requires.

**WakaTime/Hackatime TUI** — a full-featured terminal dashboard for coding activity tracking built with ratatui. The dashboard renders a 40x9 activity heatmap with streak logic, language and project breakdowns with progress bars, a leaderboard view that fetches and paginates 3669 live entries with real-time username search, a day view with hourly bar charts and date navigation, and a first-run setup flow. This is not a toy project. It handles live API fetching, cross-thread data flow, and a complex multi-pane layout that stays responsive under real usage.

Source: https://github.com/alaotach/WakaTime-TUI

**Tetris CLI** — a complete Tetris implementation in the terminal using ratatui and crossterm, with all seven tetrominoes, correct rotation states, ghost piece, three-piece lookahead preview, hard and soft drop, and score tracking. This project taught me frame-rate-independent game loops, separating input polling from state update from rendering, and keeping a tight render loop smooth without tearing. The same separation is relevant to the Miri debugger where the interpreter thread and TUI thread run concurrently.

Beyond these, I have contributed to API Dash with several merged PRs covering crash prevention, validation robustness, UI correctness, and execution history UX. I also have contributions to Hack Club's YSWS Catalog.

### 2. What is your one project or achievement you are most proud of?

The Miri TUI debugger I built before writing this proposal.

I say this not to be circular but because it represents something I have not done before: reading a large, unfamiliar, safety-critical Rust codebase, finding the right hook point, and shipping something working in a single session that goes beyond what the previous attempt at the same problem ever achieved.

Priroda, the previous Miri debugger, was a web UI that bitrotted. It had reverse debugging added once and then it died. My PoC has reverse stepping, run to main with visual fast-forward animation, output capture intercepting real program stdout and stderr through Miri's I/O shims, CFG visualization with box drawing characters, pretty-printed locals with semantic color coding, memory tracking, stack search, and mouse support. I built all of that before the proposal deadline because I wanted to know the hard parts before promising to deliver them.

That is the achievement I am most proud of this year.

### 3. What kind of problems motivate you the most?

Problems where the gap between "this should exist" and "this actually exists" is large enough that most people just accept the gap.

Miri is a perfect example. Everyone who works with Miri knows that debugging it is painful. The only tools are log output and tracing spans that produce thousands of lines. Priroda tried to fix this and died. Most people accepted that this was just how Miri debugging works. I find that kind of resigned acceptance genuinely motivating to challenge.

I have the same orientation toward other projects I work on. I am currently exploring a mobile-GPU AI inference platform, a mobile AI agent with on-device vision and device control, and a WebSocket-synced Android drawing experience. All of these exist in the space between "this should exist" and "nobody has built it properly yet."

### 4. Will you be working on GSoC full-time?

Yes, full-time. GSoC is my primary commitment during the coding period.

I have client work and side projects alongside my studies, and that is exactly why I am not worried. Client work taught me that deadlines are real, blockers need to be flagged early, and disappearing when things get hard is not an option. I bring that same accountability here.

### 5. Do you mind regularly syncing up with project mentors?

Not at all. I actively prefer it.

I learned this the hard way with my first client. They were quiet throughout the project. I kept building. When I delivered, they told me they wanted it cloud-hosted, not local. I ended up dealing with containers, Kubernetes, and deployment pipelines that were never in the original scope. That cost both of us significant time that one conversation early on would have prevented.

Regular syncs are how you avoid expensive surprises. I will bring progress openly, show what is working and what is not, and flag design questions before they become implementation dead-ends.

### 6. What interests you most about this project?

Miri is one of the most technically sophisticated tools in the Rust ecosystem and it has one of the worst debugging experiences of any major Rust tool. That gap bothers me.

When I read through the Miri source code and found the hook point in `run_threads`, I immediately understood why nobody had fixed this properly. It is not an obvious problem to solve. The interpreter state is not exposed through a clean API. The tracing output is not designed for interactive consumption. Adding a debugger means hooking into a safety-critical interpreter loop without introducing overhead when the debugger is not active.

I found that genuinely interesting to figure out. And once I had figured it out, building the TUI on top was the fun part.

### 7. Can you mention some areas where the project can be improved?

Beyond the core debugger functionality, several things would make the tool more useful in practice:

Source-level variable names are the most obvious gap. MIR locals are numbered slots. Mapping them back to source names through `var_debug_info` would make the locals pane readable for people who are not already familiar with MIR.

Borrow tracker integration is the feature that would make this genuinely unique. Miri's borrow tracker knows the complete stacked borrows or tree borrows state at every point in execution. Surfacing that means developers can watch a borrow violation happen in slow motion instead of just seeing the final error message.

Memory content inspection is the other major gap. Right now the memory pane shows allocation IDs and live/dead status. Following a pointer and seeing the actual bytes at that address, with uninit bytes highlighted, would make the tool feel like a real debugger.

CI integration is not a feature but it is the most important thing. Priroda died because it was not in CI. This project will not have the same fate.

### 8. Have you interacted with the Miri community?

I have read the Miri Zulip extensively, including the full fibers support discussion, the debugger idea thread, and discussions about the original priroda. I understand Oli's design preferences, the concern about tight coupling with borrow tracker internals, the desire for modularity so Miri contributors are not burdened by debugger maintenance, and the interest in DAP as a potential direction.

My design choices in the PoC reflect that reading. The debugger is fully opt-in behind a flag, captures state through a clean boundary rather than direct coupling to borrow tracker internals, and is structured so that adding new panes does not require touching the interpreter.

---

## Proposal Title

**Create a Debugger for Miri: A Native TUI with Step-Through Execution, Reverse Debugging, and Interpreter State Inspection**

---

## Abstract

Miri is the most powerful undefined behavior checker in the Rust ecosystem. But when Miri runs, it is a black box. You see the final error or you see nothing. The only way to understand what Miri is doing internally is to enable verbose logging and wade through thousands of lines of tracing output. Priroda tried to solve this with a web UI, worked briefly, and then bitrotted because it was too loosely coupled to Miri internals and nobody had the incentive to maintain it.

This project builds a native TUI debugger for Miri using ratatui, hooked directly into the interpreter loop, living inside the Miri repository and tested in Miri's own CI. It is not a prototype. I have already built it. This proposal is about taking what works and making it shippable, maintainable, and extended with the features that make it genuinely useful for debugging real Miri errors.

By the end of GSoC, a developer running into a Miri undefined behavior error will be able to launch the debugger, fast-forward to their code with a single keypress, step through MIR statements one at a time watching locals update and the control flow graph highlight the current basic block, step backwards through execution to find the exact moment something went wrong, inspect memory allocations in real time, and see their program's output captured in a dedicated pane. All of this while Miri's existing behavior is completely untouched when the debugger flag is not passed.

---

## 3. Detailed Description

### 3.1 The Problem

Debugging Miri errors is hard in a way that other Rust tooling is not. When the compiler gives you an error, it gives you a span, a message, and usually a suggestion. When Miri gives you an error, it gives you a stack trace and a message, but no way to understand the execution context that led there.

The standard approach is `MIRI_LOG=info`, which produces output like this:

```
INFO  rustc_const_eval::interpret::step > running statement: StorageLive(_5)
INFO  rustc_const_eval::interpret::step > running statement: _5 = copy _1
INFO  rustc_const_eval::interpret::step > running statement: StorageLive(_6)
...
```

Thousands of lines for a program that does anything nontrivial. There is no way to pause, no way to inspect locals, no way to see what the memory looks like at the point where something goes wrong.

Priroda existed to solve this. It was a web-based graphical frontend that let you step through MIR statements, view locals, and follow pointers through memory. bjorn3 even added reverse debugging. Then it stopped being maintained because it was a separate tool with a separate codebase and every time Miri internals changed, priroda broke and nobody with the knowledge to fix it had the time.

This project fixes this properly. Not a separate web UI. Not a process that communicates with Miri over a network. A native ratatui TUI that lives in the Miri repository, gets updated when Miri changes, and runs in Miri's CI so it cannot silently bitrot.

### 3.2 Why a TUI and Not DAP or a GUI

This is a real design decision and I want to address it directly.

The Miri Zulip discussion explored DAP (Debug Adapter Protocol) as a direction. Jakub Beránek noted that implementing DAP is relatively straightforward but somewhat limited, and that it can be extended to support extra information from Miri. Oli expressed interest in exploring whether DAP could work and where its limitations appear.

I chose TUI as the primary interface for three specific reasons.

First, Miri runs in the terminal. Developers using Miri are already in a terminal workflow. A TUI fits that context without switching windows or installing a VSCode extension. The workflow is: run your program under Miri, see an error, add `--debugger`, run again, step through. All of that stays in the same terminal.

Second, DAP has a fixed vocabulary: variables, scopes, stack frames, breakpoints. Miri has information that does not fit that vocabulary. Borrow tracker state, stacked borrows diagnostics, memory provenance, tree borrows violations, these are Miri-specific concepts that DAP cannot represent without extension. A TUI built directly on Miri internals can surface anything without being constrained by what DAP supports.

Third, the original priroda bitrotted because it was external. A TUI that lives in the Miri repository is modified by the same people who modify Miri's internals. When someone changes `MiriMachine`, the compiler error appears in the debugger module immediately. That structural coupling is the maintenance guarantee that priroda never had.

DAP integration is an explicit stretch goal. The TUI and a DAP layer are not mutually exclusive. The TUI becomes the primary interface and DAP enables VSCode integration as an additional output layer using the same state capture infrastructure.

### 3.3 Existing Codebase Grounding

Before writing this proposal I read the Miri source code carefully. These are the key surfaces the debugger touches.

**Execution entry point:** `src/eval.rs`, function `eval_entry`. This is where the `--debugger` flag is checked and the debugger handle is initialized before `ecx.run_threads()` is called.

**Hook point:** `src/concurrency/thread.rs`, inside `run_threads`. This is the main interpreter execution loop. The step call `this.step()?` is where each MIR statement is executed. The debugger hook fires immediately after this call, before control branches to `run_on_stack_empty`. This is the exact location where every MIR statement completes and state can be captured.

**Machine integration:** `src/machine.rs`, struct `MiriMachine`. The `debugger: Option<MiriDebuggerHandle>` field was added here. `MiriMachine::new()` sets this to `None` by default so the entire debugger path is compiled away when not active.

**I/O interception:** `src/shims/` directory. Program stdout and stderr are intercepted through Miri's existing I/O shims and routed to the debugger output channel. This is how the output pane captures `println!` and `eprintln!` output from the interpreted program.

**Tracing infrastructure:** `src/bin/log/` and `doc/tracing.md`. Miri's existing state exposure is through tracing spans with arguments exported to Perfetto. This is not suitable for interactive debugging because it is not designed for real-time consumption and does not expose a structured API. The debugger bypasses this entirely by capturing state directly from `InterpCx`.

### 3.4 What I Already Built

The PoC is not a design document. It compiles, runs, and debugs real Miri executions. Here is what it does and how it works.

#### 3.4.1 Architecture

The system has three components that communicate through `mpsc` channels.

The interpreter side runs in the main thread. After each MIR step, `MiriDebuggerHandle::send` captures the current interpreter state and sends it to the TUI thread. Then `MiriDebuggerHandle::wait_for_continue` blocks until the TUI sends a command back. When the debugger is not active this entire path is a single `None` check that disappears in release builds.

```rust
// src/concurrency/thread.rs, after this.step()?
if let Some(ref handle) = this.machine.debugger {
    let stack = this.active_thread_stack();
    if !stack.is_empty() {
        let state = DebuggerState::capture(this);
        handle.send(state);
        if handle.wait_for_continue() == DebuggerCommand::Quit {
            return Ok(());
        }
    }
}
```

The TUI thread runs independently, receiving state snapshots and rendering them with ratatui. It sends commands back to the interpreter thread in response to key events.

The channel system uses two `mpsc` channels: one for state flowing from interpreter to TUI, one for commands flowing from TUI to interpreter.

```rust
pub enum DebuggerCommand {
    Continue,
    StepOver,
    StepBack,
    RunToFrame(String),
    RunToMain,
    RunToEnd,
    Quit,
}
```

#### 3.4.2 State Capture

`DebuggerState::capture` extracts interpreter state from `MiriInterpCx` after each step:

```rust
impl DebuggerState {
    pub fn capture<'tcx>(ecx: &MiriInterpCx<'tcx>) -> Self {
        let sm = ecx.tcx.sess.source_map();
        let stack = ecx.active_thread_stack();

        let stack_frames = stack
            .iter()
            .rev()
            .map(|frame| capture_frame(sm, frame))
            .collect();

        let current_location = stack
            .last()
            .map(|frame| capture_location(ecx, frame))
            .unwrap_or(MirLocation {
                statement: "<no active frame>".to_string(),
                source_file: "<none>".to_string(),
                line: 0,
            });

        let locals = stack.last().map(capture_locals).unwrap_or_default();
        let cfg_lines = stack.last().map(capture_cfg_lines).unwrap_or_default();
        let memory = capture_memory(ecx, &locals);
        let in_user_code = is_user_code_path(&current_location.source_file);

        Self {
            current_thread: ecx.active_thread(),
            step_count: ecx.machine.basic_block_count,
            in_user_code,
            stack_frames,
            current_location,
            cfg_lines,
            locals,
            memory,
            output: ecx.machine.debugger_output.borrow().iter()
                .map(|(is_stderr, text)| OutputLine {
                    is_stderr: *is_stderr,
                    text: text.clone()
                })
                .collect(),
        }
    }
}
```

#### 3.4.3 Local Value Pretty Printing

Raw MIR local values look like `LocalState { value: Live(Immediate(Scalar(0x00000005))), ty: No }`. The PoC transforms these into human-readable values with semantic meaning:

```rust
fn prettify_local_value(raw: &str, ty: &str) -> (String, LocalKind) {
    if raw.contains("Dead") {
        return ("-".to_string(), LocalKind::Dead);
    }
    if raw.contains("Uninit") {
        return ("uninit".to_string(), LocalKind::Uninitialized);
    }
    if let Some(hex) = extract_hex_scalar(raw) {
        if is_pointer_type(ty) {
            if hex == 0 {
                return ("null".to_string(), LocalKind::Pointer);
            }
            return (format!("ptr(0x{hex:x})"), LocalKind::Pointer);
        }
        if ty == "bool" {
            return ((hex != 0).to_string(), LocalKind::Initialized);
        }
        if let Ok(num) = i128::try_from(hex) {
            return (num.to_string(), LocalKind::Initialized);
        }
    }
    (compact_debug(raw), LocalKind::Initialized)
}
```

The result is a locals table where initialized scalars show as green integers, pointers show as yellow `ptr(0x...)` values, uninitialized slots show as red `uninit`, and dead slots show as a dim dash.

#### 3.4.4 CFG Visualization

The control flow graph of the current function is extracted from the MIR body and rendered inline with box drawing characters:

```rust
fn capture_cfg_lines(frame: &Frame<'_, Provenance, FrameExtra<'_>>) -> Vec<CfgLine> {
    let current_block = match frame.current_loc() {
        Either::Left(loc) => Some(loc.block.index()),
        Either::Right(_) => None,
    };

    frame.body().basic_blocks.iter_enumerated().map(|(bb, block_data)| {
        let bb_idx = bb.index();
        let successors: Vec<usize> = block_data.terminator
            .as_ref()
            .map(|t| t.successors().map(|s| s.index()).collect())
            .unwrap_or_default();
        CfgLine {
            block: bb_idx,
            is_current: current_block == Some(bb_idx),
            successors,
        }
    }).collect()
}
```

The current basic block is highlighted in green with a filled box character. Other blocks render in dim with empty box characters. The effect is an inline control flow graph that updates on every step.

#### 3.4.5 Interpreter-Side Mode Management

`MiriDebuggerHandle` manages mode transitions on the interpreter side, deciding when to send state and when to keep running:

```rust
pub fn send(&self, state: DebuggerState) {
    let current_mode = self.mode.borrow().clone();
    match current_mode {
        DebuggerMode::Step => {
            let _ = self.state_tx.send(state);
        }
        DebuggerMode::Continue => {}
        DebuggerMode::RunToFrame(ref target) => {
            let target_lc = target.to_ascii_lowercase();
            if state.stack_frames.iter()
                .any(|frame| frame.fn_name.to_ascii_lowercase().contains(&target_lc))
            {
                *self.mode.borrow_mut() = DebuggerMode::Step;
            }
            let _ = self.state_tx.send(state);
        }
        DebuggerMode::RunToMain => {
            if state.in_user_code {
                *self.mode.borrow_mut() = DebuggerMode::Step;
            }
            let _ = self.state_tx.send(state);
        }
        DebuggerMode::RunToEnd => {
            let _ = self.state_tx.send(state);
        }
    }
}
```

In `RunToMain` mode the interpreter keeps sending state on every step (enabling the visual fast-forward animation in the TUI) but does not block waiting for input. When `in_user_code` becomes true it switches back to `Step` mode and starts blocking.

#### 3.4.6 Features Implemented in the PoC

**Step forward** through every MIR statement with `n` or space. The current MIR statement, CFG block, locals, and memory all update on every step.

**Reverse stepping** with `b`. The TUI maintains a `VecDeque<DebuggerState>` of up to 1000 snapshots. Pressing `b` replays snapshots backwards without any re-execution on the interpreter side. The status bar shows `history=312/1000` so you always know how far back you can go. This is snapshot-based rather than re-execution based, which avoids the alloc ID issues bjorn3 encountered in priroda.

**Run to main** with `m`. Fast-forwards through all stdlib initialization frames. The TUI keeps rendering each step during fast-forward so it feels like animation rather than a freeze-and-jump. User code is detected by checking whether the source path contains the rustup toolchain directory.

**Run to end** with `e`. Executes to program completion while continuing to render each step.

**Run to frame** with `p`. Select a frame in the stack pane and press `p` to run until that function appears in the call stack. Press `P` to type a function name manually.

**Stack search** with `/`. Filters the stack pane in real time as you type. Navigate between matches with `.` and `,`. Clears with `Esc`.

**Output pane**. Program stdout and stderr are intercepted through Miri's I/O shims and displayed in a dedicated pane, color coded cyan for stdout and red for stderr.

**Memory pane**. Live and deallocated allocations shown with block indicators. Pointer locals cross-referenced against the allocation table.

**Mouse support**. Scrolling with the mouse wheel works in every pane.

**Horizontal scrolling**. Every pane supports left and right scrolling for long lines.

**Status bar**. Shows current mode, step count, thread ID, focused pane, history depth, search state, and keybindings. Scrollable horizontally.

### 3.5 PoC Screenshots

The following screenshots show the debugger running on real Miri executions.

Initial state after launching with `--debugger` and pressing `m` to run to main:

PoC Screenshots -
<img width="1117" height="854" alt="Screenshot 2026-03-29 234040" src="https://github.com/user-attachments/assets/e209f950-369f-4af8-8acf-bcbbfb30f804" />
<img width="1300" height="707" alt="Screenshot 2026-03-29 171018" src="https://github.com/user-attachments/assets/22e97f62-b4cf-4b7e-8544-83a61ab6ade9" />


The stack pane shows the full call stack with the current frame highlighted in cyan. The current MIR pane shows the source file path and current MIR statement. The CFG section shows the control flow graph with the current basic block highlighted in green. The locals table shows variables with color-coded values: green for initialized scalars, yellow for pointers, red for uninit, dim dash for dead. The memory pane shows live and deallocated allocations with block indicators. The output pane shows captured program output. The status bar shows mode, step count, thread, history depth, search state, and keybindings.

The stack pane shows user-defined functions appearing alongside stdlib frames. Locals show actual integer values like `0`, `1`, `false` rather than raw debug output. The CFG shows a multi-block function with branching control flow.

### 3.6 PoC Engineering Report

#### 3.6.1 PoC Scale and Scope

The PoC was built in a single session exploring the Miri codebase from scratch. The implementation adds approximately 1500 lines across six new files and modifications to four existing files.

**New files:**
- `src/debugger/mod.rs` — `MiriDebuggerHandle`, `DebuggerCommand`, `DebuggerMode`
- `src/debugger/channel.rs` — typed channel aliases and constructors
- `src/debugger/state.rs` — `DebuggerState`, `FrameInfo`, `LocalInfo`, `LocalKind`, `CfgLine`, `MemoryInfo`, `OutputLine`, state capture functions
- `src/debugger/tui.rs` — full ratatui TUI implementation

**Modified files:**
- `src/machine.rs` — `debugger: Option<MiriDebuggerHandle>` field on `MiriMachine`
- `src/concurrency/thread.rs` — step hook in `run_threads`
- `src/eval.rs` — `debugger: bool` field on `MiriConfig`, debugger initialization in `eval_entry`
- `src/bin/miri.rs` — `--debugger` flag parsing
- `Cargo.toml` — `ratatui` and `crossterm` dependencies

#### 3.6.2 Concrete Problems Encountered and Fixed

**Problem 1: Flag passing through miri.bat**

The `--debugger` flag was initially being passed after `--` which routes it to the interpreted program rather than Miri itself. The fix is passing it via `MIRIFLAGS` environment variable or directly to the miri binary before the source file argument.

**Problem 2: Step hook firing before first frame**

The debugger was capturing state on the very first step before any stack frames were set up. The fix is a stack emptiness check before capturing: only fire the hook when `this.active_thread_stack()` is non-empty.

**Problem 3: Fast-forward animation vs teleport feel**

Initial `RunToMain` implementation switched modes and sent a continue command, causing the TUI to freeze and then snap to user code. The fix sends state on every step during fast-forward, rendering the stack flying through stdlib frames in real time, but does not block waiting for input between steps.

**Problem 4: Reverse stepping across mode boundaries**

When in reverse mode pressing `n` needed to move forward through the history snapshot deque rather than sending a step command to the interpreter. The TUI maintains a `reverse_index: Option<usize>` tracking position in the history. When `reverse_index` is `Some` and the user presses `n`, the index advances rather than sending `StepOver`. When the index reaches the end of history it resets to `None` and resumes normal stepping.

#### 3.6.3 What the PoC Does Not Have Yet

**Source-level variable names.** MIR locals are numbered slots. The `var_debug_info` field in the MIR body maps slots to source names. This mapping is not implemented yet.

**Borrow tracker state.** Stacked borrows and tree borrows diagnostics exist in Miri but are not captured in `DebuggerState`.

**Memory content inspection.** The memory pane shows allocation IDs and live/dead status but does not support following a pointer to see the bytes at that address.

**CI integration.** The debugger is not yet in Miri's CI pipeline.

**DAP layer.** Planned as a stretch goal.

#### 3.6.4 Technical Debt to Address

The PoC proved the UX and core architecture work, but deliberately took shortcuts that must be rewritten for production readiness:

**Robust Value Extraction:** The PoC relies on formatting MIR locals to debugging strings (`format!("{local:?}")`) and running brittle string operations. For production, value extraction will use Miri/Rustc's actual evaluation APIs (e.g., `ecx.read_immediate()` and typed MIR variants).
**Fast-Forward Performance Optimization:** In `RunToMain` and `RunToEnd`, the PoC currently captures the full memory and locals state on every single MIR step, causing massive allocation overhead. This will be optimized by throttling full state snapshots or capturing only the `current_location` during fast-forward animations.
**Reliable "User Code" Detection:** The current check for user code relies on string-matching the file path for `.rustup/toolchains/miri`. This will be replaced with robust `rustc` compiler APIs, such as using `tcx.is_local(instance.def_id())` to definitively differentiate workspace code from `std` library internals.
**Advanced CFG Terminator Visualization:** The inline CFG visualization currently strings together simple successors (`bb2 -> bb3, bb4`). This will be enhanced to evaluate terminators and label exact branching conditions (like `switchInt` boundaries) in the UI.
**Graceful Thread Teardown:** The PoC's channel handling swallows dropped messages with `.unwrap_or()`. Production hardening will ensure robust cross-thread error boundaries—if the TUI thread crashes, the Miri interpreter cleanly aborts execution.

### 3.7 UI and UX Design

The TUI uses a five-pane layout with a status bar.

**Left pane:** Stack frames list. Shows function name in light cyan and source file path in dim gray for each frame. Current frame highlighted with solid cyan background. Search active frames with `/`.

**Top right:** Current MIR pane. Shows source file path and line number, current MIR statement in light cyan, and CFG visualization below. Current basic block highlighted green, others dim.

**Middle right:** Locals table with Local, Type, Value columns. Values color-coded by `LocalKind`: green for initialized scalars, yellow for pointers, red for uninit, dim for dead.

**Lower right:** Memory pane. Allocations shown with filled or empty block indicators, cyan for live, dim for deallocated.

**Bottom right:** Output pane. Program stdout in light cyan, stderr in red. Accumulates across entire execution.

**Status bar:** Cyan background with black text. Shows `mode=step steps=22 thread=0 focus=stack history=312/1000 search=off` followed by full keybinding reference. Horizontally scrollable with `[` and `]`.

All panes support up/down and left/right scrolling with arrow keys and mouse wheel.

### 3.8 History and Persistence Strategy

The reverse stepping history uses a `VecDeque<DebuggerState>` with a capacity of 1000 snapshots. This is snapshot-based rather than re-execution based for two reasons: re-execution requires running N-1 steps to go back one step which is slow at large step counts, and re-execution has issues with global alloc IDs that bjorn3 documented in the original priroda implementation.

The tradeoff is memory: each `DebuggerState` snapshot contains the full stack frame list, locals for each frame, memory summary, and output buffer. For programs developers typically debug with Miri this is acceptable. The capacity limit is configurable and the status bar shows current usage.

### 3.9 Testing Strategy

Testing is staged by milestone, not saved for the end.

**Unit tests** cover state capture correctness, local value pretty printing edge cases, CFG extraction, user code path detection, and snapshot history deque boundary conditions.

**Integration tests** run the debugger against test programs in `tests/pass/` and verify that stepping through a program does not panic, that `RunToMain` correctly identifies user code, that the output pane captures program output, and that reverse stepping history is consistent.

**Regression safety** runs the full existing Miri test suite on every PR that touches the debugger module to verify that the `--debugger` flag presence or absence does not affect program interpretation.

### 3.10 Reliability and Risk Management

**Risk 1: Miri internal API changes**

Miri changes frequently. The debugger is designed with clean boundaries between the state capture layer and the TUI layer so that internal changes only affect `state.rs` and not the rest of the debugger. CI integration ensures changes that break the debugger surface immediately rather than silently.

**Risk 2: Borrow tracker coupling**

The Zulip discussion flagged concern about too tight coupling with tree and stacked borrows data. The design keeps borrow tracker state as an optional addition to `DebuggerState` behind a feature boundary, so changes to borrow tracker internals do not require changes to the core stepping and rendering infrastructure.

**Risk 3: Performance impact on non-debugger Miri runs**

The entire hook is behind `if let Some(ref handle) = this.machine.debugger` where `debugger` is `None` by default. This is a single comparison against `None` that the compiler should eliminate in release builds.

**Risk 4: Scope**

Six milestones in twelve weeks with a working PoC as the starting point is achievable. The hard architectural questions are answered. The risk is in the details of milestone 3 (borrow tracker integration) which requires careful interface design. If that milestone runs over time, milestone 4 (memory content inspection) can be deferred as it is independently useful but not a blocker for the core experience.

### 3.11 Deliverables Summary

**Core**

1. Debugger module stabilized, cleaned up, and merged into Miri repository
2. CI integration with tests that prevent bitrotting
3. Source-level variable names in locals pane
4. Borrow tracker state surfaced in dedicated pane
5. Memory content inspection with pointer following
6. Documentation for users and contributors

**Stretch**

1. DAP server layer enabling VSCode integration
2. Breakpoint support with function name and source location targeting
3. Expanded memory visualization including struct field layout

---

## 4. Weekly Timeline

### Week 1: Stabilization and first PR

Clean up the existing PoC code to Miri's code style and quality expectations. Handle edge cases: programs that terminate immediately, programs with no user code, multi-threaded programs, programs that panic. Open a draft PR for mentor review. Establish the code review cadence and confirm milestone acceptance criteria.

**Deliverable:** Draft PR open with cleaned-up debugger module. All existing Miri tests pass.

### Week 2: CI integration

Write tests that run in Miri's CI pipeline. Tests verify that the debugger compiles, that `--debugger` flag parsing works, that basic stepping through a simple program does not panic, that `RunToMain` correctly stops at user code, and that the output pane captures program output. Add a CI job that runs these tests on every push.

**Deliverable:** Debugger tested in CI. First PR ready for review.

### Week 3: Multi-threaded and edge case handling

Test and fix the debugger against multi-threaded programs that Miri handles specially. Handle programs that finish before the first step, programs that immediately panic, programs with very deep call stacks. Ensure the final-state display works correctly so the TUI stays open showing the last snapshot after the program terminates.

**Deliverable:** Debugger handles the full range of programs in `tests/pass/` without panicking.

### Week 4: Source-level variable name research and prototype

Read the `var_debug_info` structure in MIR bodies. Understand how MIR locals map to source variable names, handling closures, compiler-generated temporaries, and variables that span multiple MIR locals. Build a prototype mapping for simple cases.

**Deliverable:** Prototype that correctly maps MIR slot numbers to source names for straightforward functions.

### Week 5: Source-level variable names complete

Complete the `var_debug_info` mapping including closures, temporaries, and variables that appear in multiple scopes. Update the locals pane to show source names with MIR slot number as fallback. Test against a variety of real programs.

**Deliverable:** Locals pane shows source variable names. PR open for review.

### Week 6: Source names stabilization and PR merge

Address review feedback on source name mapping. Fix edge cases discovered during testing. Update documentation for the locals pane. Merge PR.

**Deliverable:** Source-level variable names merged.

### Week 7: Borrow tracker interface design

Research the borrow tracker internal APIs for stacked borrows and tree borrows. Design the interface between borrow tracker state and `DebuggerState` to minimize coupling. The goal is a clean extraction layer that does not break when borrow tracker internals change. Discuss design with Miri maintainers before implementation.

**Deliverable:** Design document for borrow tracker integration reviewed and approved.

### Week 8: Borrow tracker stacked borrows integration

Implement borrow state capture for the stacked borrows model. Add a new pane or extend the memory pane to show borrow state for the currently focused pointer. Test against programs that trigger stacked borrows violations and verify the debugger shows the violation before Miri reports the error.

**Deliverable:** Stacked borrows state visible in debugger. PR open for review.

### Week 9: Borrow tracker tree borrows integration

Extend borrow state capture for the tree borrows model. Ensure the display works correctly when switching between stacked borrows and tree borrows modes via Miri flags. Handle programs that trigger tree borrows violations.

**Deliverable:** Both borrow tracker models surfaced. PR ready for merge.

### Week 10: Memory content inspection

Implement pointer following in the memory pane. Select a pointer local and press enter to see the bytes at that memory address, with uninit bytes highlighted in red and initialized bytes interpreted based on the pointed-to type. Handle nested pointers, struct fields, and array elements.

**Deliverable:** Memory content inspection working for simple pointer types. PR open.

### Week 11: Memory inspection polish and complex types

Extend memory content inspection to handle structs, enums, arrays, and nested pointers. Add field-level display for struct types using the type information available from the MIR body. Test against programs with complex data structures.

**Deliverable:** Memory inspection handles complex types. PR ready for review.

### Week 12: Documentation, final polish, and submission

Write user documentation covering how to launch the debugger, all keybindings, and common debugging workflows. Write contributor documentation covering the module architecture, how to add new panes, and how to update the debugger when Miri internals change. Final UX consistency pass. Close any open review comments. Prepare final evaluation artifacts.

**Deliverable:** Complete, review-ready debugger with full documentation.

---

## 5. Prior Work

### WakaTime/Hackatime TUI

Before this proposal I built a full-featured terminal dashboard for Hackatime, the Hack Club coding activity tracker. This is production-quality Rust with multiple views, live API fetching, and a level of visual polish that takes real effort to achieve in a terminal environment.

The dashboard view shows weekly coding activity as a 40x9 heatmap where cell brightness maps to activity intensity, language and project breakdowns with progress bars, and a live coding streak tracker with current and longest streak displayed.

![Dashboard](https://github.com/user-attachments/assets/ee2667da-4fd3-46d5-99cf-0dac72d893cf)

The leaderboard view fetches the live Hackatime global leaderboard with 3669 entries, supports daily and weekly tabs, real-time username search with filtered results, and highlights your own position in the rankings.

![Leaderboard](https://github.com/user-attachments/assets/76a14e35-5bf7-47eb-811e-0db6667f9485)

The day view shows an hourly bar chart of coding activity for any date, with date navigation, jump-to-date input, and statistics like total time, max hour, and most productive hour.

![Day View](https://github.com/user-attachments/assets/63e9eaf1-ff49-402c-83aa-de5b514030ba)

The projects view fetches all projects with time tracked, heartbeat count, and language breakdown, handling 100+ projects cleanly with scrolling and search.

![Projects](https://github.com/user-attachments/assets/325e94ec-ddf1-4ac2-9230-1b4961996141)

The setup screen handles first-run configuration gracefully, detecting missing API keys and walking users through setup rather than crashing.

![Setup](https://github.com/user-attachments/assets/cbaa2c9b-7efe-4c55-83e7-d5c4c8173771)

This project is what made me confident I could build the Miri debugger TUI. The rendering patterns, event loop design, multi-pane layout, and cross-thread data flow I developed here translate directly. The difference is that instead of fetching data from an HTTP API, the debugger receives data from an `mpsc` channel connected to the Miri interpreter loop. The ratatui side is the same problem.

Source: https://github.com/alaotach/WakaTime-TUI

### Tetris CLI

I also built a complete Tetris implementation in the terminal using ratatui and crossterm. This is more demanding from a rendering perspective than the WakaTime TUI because Tetris requires precise frame timing and smooth animation. The implementation has all seven tetrominoes with correct rotation states, ghost piece, three-piece lookahead preview with manual selection, hard and soft drop, and score tracking.

What this project taught me that the WakaTime TUI did not is how to handle a tight render loop where timing matters independent of keyboard input. That same separation of input polling, state update, and rendering is directly relevant to the Miri debugger where the interpreter thread and TUI thread run concurrently and need to stay synchronized.

Both projects are written in the same style I used for the Miri debugger PoC: flat inline rendering logic inside the `terminal.draw` closure, explicit state structs rather than component abstractions, and `mpsc` channels for cross-thread communication. The Miri debugger is architecturally continuous with both of these projects.

---

## 6. Why I Can Execute This

I want to be direct rather than list generic skills.

I already built this. The PoC runs. It has stepping, reverse stepping, run to main with visual animation, output capture, CFG visualization, pretty-printed locals with semantic color coding, memory tracking, stack search, and mouse support. I built all of that before this deadline because I wanted to understand the hard parts before promising to deliver them.

The specific technical things this project requires: reading an unfamiliar safety-critical Rust codebase and finding the right hook point without breaking anything, building a multi-pane ratatui TUI with cross-thread communication, making interpreter state human-readable, implementing snapshot-based reverse stepping. I have done all of these already.

I am in my second year at JNU studying Electronics and Communication Engineering, doing freelance development work and building side projects alongside my studies. I have five years of coding experience in Rust, Python, TypeScript, JavaScript, and several other languages. I have the time, the technical foundation, and genuine interest in this project specifically.

Miri is the most powerful UB checker in the Rust ecosystem and it deserves a debugger that does not bitrot. That is the work I want to do this summer.

---

## 7. Additional Notes

The debugger is fully opt-in. When `--debugger` is not passed, `MiriMachine.debugger` is `None` and the entire hook path is a single `None` check that should be eliminated by the compiler in release builds. Existing Miri behavior is completely untouched.

The module structure is designed for minimal maintenance burden on Miri maintainers. New panes are added by extending `DebuggerState` and adding a render function in `tui.rs`. Changes to Miri internals that break state capture surface as compiler errors in `state.rs` rather than silent runtime failures. CI tests catch regressions before they reach main.

I am comfortable with scope adjustments based on mentor feedback. The milestone ordering reflects my current best judgment about what is most valuable and what is most risky, but I will adapt that ordering based on what mentors and the Miri team think matters most.