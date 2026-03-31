use rustc_data_structures::either::Either;

use crate::*;

#[derive(Clone, Debug)]
pub struct FrameInfo {
	pub fn_name: String,
	pub source_file: String,
	pub line: u32,
	pub locals: Vec<LocalInfo>,
}

#[derive(Clone, Debug)]
pub struct MirLocation {
	pub statement: String,
	pub source_file: String,
	pub line: u32,
}

#[derive(Clone, Debug)]
pub struct LocalInfo {
	pub name: String,
	pub value: String,
	pub ty: String,
	pub kind: LocalKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
	Initialized,
	Pointer,
	Uninitialized,
	Dead,
}

#[derive(Clone, Debug)]
pub struct CfgLine {
	pub block: usize,
	pub text: String,
	pub is_current: bool,
	pub successors: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct MemoryInfo {
	pub name: String,
	pub detail: String,
}

#[derive(Clone, Debug)]
pub struct OutputLine {
	pub is_stderr: bool,
	pub text: String,
}

#[derive(Clone, Debug)]
pub struct DebuggerState {
	pub current_thread: ThreadId,
	pub step_count: u64,
	pub in_user_code: bool,
	pub stack_frames: Vec<FrameInfo>,
	pub current_location: MirLocation,
	pub cfg_lines: Vec<CfgLine>,
	pub locals: Vec<LocalInfo>,
	pub memory: Vec<MemoryInfo>,
	pub output: Vec<OutputLine>,
}

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
		let output = ecx
			.machine
			.debugger_output
			.borrow()
			.iter()
			.map(|(is_stderr, text)| OutputLine { is_stderr: *is_stderr, text: text.clone() })
			.collect();
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
			output,
		}
	}
}

fn is_user_code_path(path: &str) -> bool {
	let lower = path.to_ascii_lowercase();
	if lower.contains(".rustup\\toolchains\\miri") || lower.contains(".rustup/toolchains/miri") {
		return false;
	}
	if path.starts_with('<') {
		return false;
	}
	true
}

fn capture_locals(frame: &Frame<'_, Provenance, FrameExtra<'_>>) -> Vec<LocalInfo> {
	frame
		.locals
		.iter()
		.enumerate()
		.map(|(idx, local)| {
			let local_idx = rustc_middle::mir::Local::from_usize(idx);
			let local_decl = &frame.body().local_decls[local_idx];
			let raw = format!("{local:?}");
			let (value, kind) = prettify_local_value(&raw, &local_decl.ty.to_string());
			LocalInfo {
				name: format!("_{idx}"),
				value,
				ty: local_decl.ty.to_string(),
				kind,
			}
		})
		.collect()
}

fn capture_frame(
	sm: &rustc_span::source_map::SourceMap,
	frame: &Frame<'_, Provenance, FrameExtra<'_>>,
) -> FrameInfo {
	let span = frame.current_span();
	let pos = sm.lookup_char_pos(span.lo());
	FrameInfo {
		fn_name: frame.instance().to_string(),
		source_file: pos.file.name.prefer_local_unconditionally().to_string(),
		line: u32::try_from(pos.line).unwrap_or(u32::MAX),
		locals: capture_locals(frame),
	}
}

fn capture_cfg_lines(frame: &Frame<'_, Provenance, FrameExtra<'_>>) -> Vec<CfgLine> {
	let current_block = match frame.current_loc() {
		Either::Left(loc) => Some(loc.block.index()),
		Either::Right(_) => None,
	};

	frame
		.body()
		.basic_blocks
		.iter_enumerated()
		.map(|(bb, block_data)| {
			let bb_idx = bb.index();
			let mut text = format!("bb{bb_idx}");
			let mut successors = Vec::new();
			if let Some(term) = &block_data.terminator {
				let succs: Vec<usize> = term
					.successors()
					.map(|target| target.index())
					.collect();
				successors = succs.clone();
				if succs.is_empty() {
					text.push_str(" -> <end>");
				} else {
					text.push_str(" -> ");
					text.push_str(
						&succs.iter().map(|s| format!("bb{s}")).collect::<Vec<_>>().join(", "),
					);
				}
			}
			CfgLine {
				block: bb_idx,
				text,
				is_current: current_block == Some(bb_idx),
				successors,
			}
		})
		.collect()
}

fn capture_memory(ecx: &MiriInterpCx<'_>, locals: &[LocalInfo]) -> Vec<MemoryInfo> {
	let mut entries = Vec::new();

	let alloc_spans = ecx.machine.allocation_spans.borrow();
	entries.push(MemoryInfo {
		name: "allocations".to_string(),
		detail: alloc_spans.len().to_string(),
	});

	for (alloc_id, (_alloc, dealloc)) in alloc_spans.iter().take(32) {
		entries.push(MemoryInfo {
			name: format!("{alloc_id:?}"),
			detail: if dealloc.is_some() { "deallocated" } else { "live" }.to_string(),
		});
	}

	for local in locals.iter().filter(|l| l.kind == LocalKind::Pointer).take(16) {
		entries.push(MemoryInfo {
			name: format!("ptr {}", local.name),
			detail: local.value.clone(),
		});
	}

	entries
}

fn prettify_local_value(raw: &str, ty: &str) -> (String, LocalKind) {
	let lower = raw.to_ascii_lowercase();

	if lower.contains("dead") {
		return ("-".to_string(), LocalKind::Dead);
	}
	if lower.contains("uninit") {
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
		return (format!("0x{hex:x}"), LocalKind::Initialized);
	}

	if is_pointer_type(ty) {
		return (compact_debug(raw), LocalKind::Pointer);
	}

	(compact_debug(raw), LocalKind::Initialized)
}

fn extract_hex_scalar(raw: &str) -> Option<u128> {
	let scalar_pos = raw.find("Scalar(")?;
	let tail = &raw[scalar_pos..];
	let start = tail.find("0x")? + scalar_pos;
	let rest = &raw[start + 2..];
	let hex_len = rest
		.chars()
		.take_while(|c| c.is_ascii_hexdigit())
		.count();
	if hex_len == 0 {
		return None;
	}
	u128::from_str_radix(&rest[..hex_len], 16).ok()
}

fn is_pointer_type(ty: &str) -> bool {
	ty.contains('*') || ty.contains('&')
}

fn compact_debug(raw: &str) -> String {
	raw
		.replace("LocalState { value: Live(", "")
		.replace("), ty: No }", "")
		.replace("Immediate(", "")
		.replace("Scalar(", "")
		.trim()
		.to_string()
}

fn capture_location(
	ecx: &MiriInterpCx<'_>,
	frame: &Frame<'_, Provenance, FrameExtra<'_>>,
) -> MirLocation {
	let sm = ecx.tcx.sess.source_map();
	let current_span = frame.current_span();
	let char_pos = sm.lookup_char_pos(current_span.lo());

	let statement = match frame.current_loc() {
		Either::Left(loc) => {
			let block = &frame.body().basic_blocks[loc.block];
			if let Some(stmt) = block.statements.get(loc.statement_index) {
				format!("{stmt:?}")
			} else if let Some(term) = &block.terminator {
				format!("{:?}", term.kind)
			} else {
				"<no statement>".to_string()
			}
		}
		Either::Right(span) => {
			let span = sm.span_to_diagnostic_string(span);
			format!("external span: {span}")
		}
	};

	MirLocation {
		statement,
		source_file: char_pos.file.name.prefer_local_unconditionally().to_string(),
		line: u32::try_from(char_pos.line).unwrap_or(u32::MAX),
	}
}
