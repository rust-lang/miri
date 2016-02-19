use rustc::middle::{const_eval, def_id, ty};
use rustc::middle::cstore::CrateStore;
use rustc::mir::repr::{self as mir, Mir};
use rustc::mir::mir_map::MirMap;
use syntax::ast::Attribute;
use syntax::attr::AttrMetaMethods;

use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
enum Value {
    Uninit,
    Bool(bool),
    Int(i64), // FIXME(tsion): Should be bit-width aware.
    Pointer(Pointer),
    Adt { variant: usize, data_ptr: Pointer },
    Func(def_id::DefId),
    Aggregate(Vec<Value>),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum PointerKind {
    Stack { frame: usize, stack: usize },
    Heap(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Pointer {
    projection: Vec<usize>,
    kind: PointerKind,
}

impl Pointer {
    fn offset(&self, offset: usize) -> Pointer {
        Pointer {
            projection: self.projection.iter().cloned().chain(Some(offset)).collect(),
            kind: self.kind,
        }
    }

    fn stack(frame: usize, stack: usize) -> Pointer {
        Pointer {
            projection: Vec::new(),
            kind: PointerKind::Stack { frame: frame, stack: stack },
        }
    }

    fn heap(ptr: usize) -> Pointer {
        Pointer {
            projection: Vec::new(),
            kind: PointerKind::Heap(ptr),
        }
    }
}

/// A stack frame:
///
/// ```text
/// +-----------------------+
/// | Arg(0)                |
/// | Arg(1)                | arguments
/// | ...                   |
/// | Arg(num_args - 1)     |
/// + - - - - - - - - - - - +
/// | Var(0)                |
/// | Var(1)                | variables
/// | ...                   |
/// | Var(num_vars - 1)     |
/// + - - - - - - - - - - - +
/// | Temp(0)               |
/// | Temp(1)               | temporaries
/// | ...                   |
/// | Temp(num_temps - 1)   |
/// + - - - - - - - - - - - +
/// | Aggregates            | aggregates
/// +-----------------------+
/// ```
#[derive(Debug)]
struct Frame {
    /// A pointer to a stack cell to write the return value of the current call, if it's not a
    /// diverging call.
    return_ptr: Option<Pointer>,
    stack: Vec<Value>,

    num_args: usize,
    num_vars: usize,
    num_temps: usize,

    id: usize,
}

impl Frame {
    fn arg_offset(&self, i: usize) -> usize {
        i
    }

    fn var_offset(&self, i: usize) -> usize {
        self.num_args + i
    }

    fn temp_offset(&self, i: usize) -> usize {
        self.num_args + self.num_vars + i
    }

    fn stack_alloc(&mut self) -> Pointer {
        let ptr = Pointer::stack(self.id, self.stack.len());
        self.stack.push(Value::Uninit);
        ptr
    }

    fn stack_ptr(&self, idx: usize) -> Pointer {
        Pointer::stack(self.id, idx)
    }
}

struct Interpreter<'a, 'tcx: 'a> {
    tcx: &'a ty::ctxt<'tcx>,
    mir_map: &'a MirMap<'tcx>,
    call_stack: Vec<Frame>,
    heap: HashMap<usize, Value>,
    heap_idx: usize,
}

impl<'a, 'tcx> Interpreter<'a, 'tcx> {
    fn new(tcx: &'a ty::ctxt<'tcx>, mir_map: &'a MirMap<'tcx>) -> Self {
        Interpreter {
            tcx: tcx,
            mir_map: mir_map,
            call_stack: Vec::new(),
            heap: HashMap::new(),
            heap_idx: 1,
        }
    }

    fn push_stack_frame(&mut self, mir: &Mir, args: &[Value], return_ptr: Option<Pointer>) {
        let mut frame = Frame {
            return_ptr: return_ptr,
            num_args: mir.arg_decls.len(),
            num_vars: mir.var_decls.len(),
            num_temps: mir.temp_decls.len(),
            stack: vec![Value::Uninit; mir.arg_decls.len() + mir.var_decls.len() + mir.temp_decls.len()],
            id: self.call_stack.len(),
        };

        for (i, arg) in args.iter().enumerate() {
            let i = frame.arg_offset(i);
            frame.stack[i] = arg.clone();
        }

        self.call_stack.push(frame);

    }

    fn pop_stack_frame(&mut self) {
        self.call_stack.pop().expect("tried to pop stack frame, but there were none");
    }

    fn call(&mut self, mir: &Mir, args: &[Value], return_ptr: Option<Pointer>) {
        debug!("call");
        self.push_stack_frame(mir, args, return_ptr);
        let mut block = mir::START_BLOCK;

        loop {
            debug!("Entering block: {:?}", block);
            let block_data = mir.basic_block_data(block);

            for stmt in &block_data.statements {
                debug!("{:?}", stmt);

                match stmt.kind {
                    mir::StatementKind::Assign(ref lvalue, ref rvalue) => {
                        let ptr = self.eval_lvalue(lvalue);
                        let value = self.eval_rvalue(rvalue);
                        self.write_pointer(ptr, value);
                    }
                }
            }

            debug!("{:?}", block_data.terminator());

            match *block_data.terminator() {
                mir::Terminator::Return => break,
                mir::Terminator::Goto { target } => block = target,

                mir::Terminator::Call { ref func, ref args, ref destination, .. } => {
                    let ptr = destination.as_ref().map(|&(ref lv, _)| self.eval_lvalue(lv));
                    let func_val = self.eval_operand(func);

                    if let Value::Func(def_id) = func_val {
                        let mir_data;
                        let mir = match self.tcx.map.as_local_node_id(def_id) {
                            Some(node_id) => self.mir_map.map.get(&node_id).unwrap(),
                            None => {
                                let cstore = &self.tcx.sess.cstore;
                                mir_data = cstore.maybe_get_item_mir(self.tcx, def_id).unwrap();
                                &mir_data
                            }
                        };

                        let arg_vals: Vec<Value> =
                            args.iter().map(|arg| self.eval_operand(arg)).collect();

                        self.call(mir, &arg_vals, ptr);

                        if let Some((_, target)) = *destination {
                            block = target;
                        }
                    } else {
                        panic!("tried to call a non-function value: {:?}", func_val);
                    }
                }

                mir::Terminator::If { ref cond, targets: (then_target, else_target) } => {
                    match self.eval_operand(cond) {
                        Value::Bool(true) => block = then_target,
                        Value::Bool(false) => block = else_target,
                        cond_val => panic!("Non-boolean `if` condition value: {:?}", cond_val),
                    }
                }

                mir::Terminator::SwitchInt { ref discr, ref values, ref targets, .. } => {
                    let discr_val = self.read_lvalue(discr);

                    let index = values.iter().position(|v| discr_val == self.eval_constant(v))
                        .expect("discriminant matched no values");

                    block = targets[index];
                }

                mir::Terminator::Switch { ref discr, ref targets, .. } => {
                    let discr_val = self.read_lvalue(discr);

                    if let Value::Adt { variant, .. } = discr_val {
                        block = targets[variant];
                    } else {
                        panic!("Switch on non-Adt value: {:?}", discr_val);
                    }
                }

                mir::Terminator::Drop { target, .. } => {
                    // TODO: Handle destructors and dynamic drop.
                    block = target;
                }

                mir::Terminator::Resume => unimplemented!(),
            }
        }

        self.pop_stack_frame();
    }

    fn frame(&self) -> &Frame {
        self.call_stack.last().expect("missing call frame")
    }

    fn frame_mut(&mut self) -> &mut Frame {
        self.call_stack.last_mut().expect("missing call frame")
    }

    fn eval_lvalue(&self, lvalue: &mir::Lvalue) -> Pointer {
        let frame = self.frame();

        match *lvalue {
            mir::Lvalue::ReturnPointer =>
                frame.return_ptr
                     .as_ref()
                     .expect("ReturnPointer used in a function with no return value")
                     .clone(),
            mir::Lvalue::Arg(i)  => frame.stack_ptr(frame.arg_offset(i as usize)),
            mir::Lvalue::Var(i)  => frame.stack_ptr(frame.var_offset(i as usize)),
            mir::Lvalue::Temp(i) => frame.stack_ptr(frame.temp_offset(i as usize)),

            mir::Lvalue::Projection(ref proj) => {
                // proj.base: Lvalue
                // proj.elem: ProjectionElem<Operand>

                let base_ptr = self.eval_lvalue(&proj.base);

                match proj.elem {
                    mir::ProjectionElem::Field(field) => {
                        debug!("field index: {:?}", field);
                        base_ptr.offset(field.index())
                    }

                    mir::ProjectionElem::Downcast(_, variant) => {
                        debug!("downcast: {:?}", variant);
                        let adt_val = self.read_pointer(base_ptr);
                        if let Value::Adt { variant: actual_variant, data_ptr } = adt_val {
                            debug_assert_eq!(variant, actual_variant);
                            data_ptr
                        } else {
                            panic!("Downcast attempted on non-ADT: {:?}", adt_val)
                        }
                    }

                    mir::ProjectionElem::Deref => {
                        debug!("deref");
                        let ptr_val = self.read_pointer(base_ptr);
                        if let Value::Pointer(ptr) = ptr_val {
                            ptr
                        } else {
                            panic!("Deref attempted on non-pointer: {:?}", ptr_val)
                        }
                    }

                    mir::ProjectionElem::Index(ref _operand) => unimplemented!(),
                    mir::ProjectionElem::ConstantIndex { .. } => unimplemented!(),
                }
            }

            _ => unimplemented!(),
        }
    }

    fn eval_binary_op(&mut self, bin_op: mir::BinOp, left: Value, right: Value) -> Value {
        match (left, right) {
            (Value::Int(l), Value::Int(r)) => {
                match bin_op {
                    mir::BinOp::Add    => Value::Int(l + r),
                    mir::BinOp::Sub    => Value::Int(l - r),
                    mir::BinOp::Mul    => Value::Int(l * r),
                    mir::BinOp::Div    => Value::Int(l / r),
                    mir::BinOp::Rem    => Value::Int(l % r),
                    mir::BinOp::BitXor => Value::Int(l ^ r),
                    mir::BinOp::BitAnd => Value::Int(l & r),
                    mir::BinOp::BitOr  => Value::Int(l | r),
                    mir::BinOp::Shl    => Value::Int(l << r),
                    mir::BinOp::Shr    => Value::Int(l >> r),
                    mir::BinOp::Eq     => Value::Bool(l == r),
                    mir::BinOp::Lt     => Value::Bool(l < r),
                    mir::BinOp::Le     => Value::Bool(l <= r),
                    mir::BinOp::Ne     => Value::Bool(l != r),
                    mir::BinOp::Ge     => Value::Bool(l >= r),
                    mir::BinOp::Gt     => Value::Bool(l > r),
                }
            }

            _ => unimplemented!(),
        }
    }

    fn eval_rvalue(&mut self, rvalue: &mir::Rvalue) -> Value {
        debug!("eval_rvalue: {:?}", rvalue);
        match *rvalue {
            mir::Rvalue::Use(ref operand) => self.eval_operand(operand),

            mir::Rvalue::BinaryOp(bin_op, ref left, ref right) => {
                let left_val = self.eval_operand(left);
                let right_val = self.eval_operand(right);
                self.eval_binary_op(bin_op, left_val, right_val)
            }

            mir::Rvalue::UnaryOp(un_op, ref operand) => {
                match (un_op, self.eval_operand(operand)) {
                    (mir::UnOp::Not, Value::Int(n)) => Value::Int(!n),
                    (mir::UnOp::Neg, Value::Int(n)) => Value::Int(-n),
                    _ => unimplemented!(),
                }
            }

            mir::Rvalue::Ref(_region, _kind, ref lvalue) => {
                Value::Pointer(self.eval_lvalue(lvalue))
            }

            mir::Rvalue::Aggregate(mir::AggregateKind::Adt(_adt_def, variant, _substs),
                                   ref operands) => {
                let operands = operands.iter().map(|operand| self.eval_operand(operand)).collect();
                let ptr = self.frame_mut().stack_alloc();
                self.write_pointer(ptr.clone(), Value::Aggregate(operands));

                Value::Adt { variant: variant, data_ptr: ptr }
            }

            mir::Rvalue::Box(_) => Value::Pointer(self.heap_alloc()),

            ref r => panic!("can't handle rvalue: {:?}", r),
        }
    }

    fn eval_operand(&mut self, op: &mir::Operand) -> Value {
        match *op {
            mir::Operand::Consume(ref lvalue) => self.read_lvalue(lvalue),

            mir::Operand::Constant(ref constant) => {
                match constant.literal {
                    mir::Literal::Value { ref value } => self.eval_constant(value),

                    mir::Literal::Item { def_id, kind, .. } => match kind {
                        mir::ItemKind::Function | mir::ItemKind::Method => Value::Func(def_id),
                        _ => panic!("can't handle item literal: {:?}", constant.literal),
                    },
                }
            }
        }
    }

    fn eval_constant(&self, const_val: &const_eval::ConstVal) -> Value {
        match *const_val {
            const_eval::ConstVal::Float(_f)         => unimplemented!(),
            const_eval::ConstVal::Int(i)            => Value::Int(i),
            const_eval::ConstVal::Uint(_u)          => unimplemented!(),
            const_eval::ConstVal::Str(ref _s)       => unimplemented!(),
            const_eval::ConstVal::ByteStr(ref _bs)  => unimplemented!(),
            const_eval::ConstVal::Bool(b)           => Value::Bool(b),
            const_eval::ConstVal::Struct(_node_id)  => unimplemented!(),
            const_eval::ConstVal::Tuple(_node_id)   => unimplemented!(),
            const_eval::ConstVal::Function(_def_id) => unimplemented!(),
            const_eval::ConstVal::Array(_, _)       => unimplemented!(),
            const_eval::ConstVal::Repeat(_, _)      => unimplemented!(),
        }
    }

    fn read_lvalue(&self, lvalue: &mir::Lvalue) -> Value {
        debug!("read_lvalue: {:?}", lvalue);
        self.read_pointer(self.eval_lvalue(lvalue))
    }

    fn read_pointer(&self, p: Pointer) -> Value {
        debug!("read_pointer: {:?}", p);
        let mut val = match p.kind {
            PointerKind::Stack{ frame, stack } => &self.call_stack[frame].stack[stack],
            PointerKind::Heap(idx) => {
                debug_assert!(idx < self.heap_idx, "use before alloc");
                self.heap.get(&idx).expect("use after free")
            },
        };
        for offset in &p.projection {
            if let Value::Aggregate(ref v) = *val {
                val = &v[*offset];
            } else {
                panic!("tried to offset a non-aggregate");
            }
        }
        if let Value::Uninit = *val {
            panic!("reading uninitialized value at {:?}", p);
        }
        val.clone()
    }

    fn write_pointer(&mut self, p: Pointer, val: Value) {
        match p.kind {
            PointerKind::Stack{ frame, stack } => self.call_stack[frame].stack[stack] = val,
            PointerKind::Heap(idx) => {
                debug_assert!(idx < self.heap_idx, "use before alloc");
                *self.heap.get_mut(&idx).expect("use after free") = val;
            },

        }
    }

    fn heap_alloc(&mut self) -> Pointer {
        let idx = self.heap_idx;
        self.heap_idx += 1;
        assert!(self.heap.insert(idx, Value::Uninit).is_none());
        Pointer::heap(idx)
    }
}

pub fn interpret_start_points<'tcx>(tcx: &ty::ctxt<'tcx>, mir_map: &MirMap<'tcx>) {
    for (&id, mir) in &mir_map.map {
        for attr in tcx.map.attrs(id) {
            if attr.check_name("miri_run") {
                let item = tcx.map.expect_item(id);

                print!("Interpreting: {}... ", item.name);

                let mut interpreter = Interpreter::new(tcx, mir_map);
                let return_ptr = interpreter.heap_alloc();
                interpreter.call(mir, &[], Some(return_ptr.clone()));

                let val_str = format!("{:?}", interpreter.read_pointer(return_ptr));
                if !check_expected(&val_str, attr) {
                    println!("=> {}\n", val_str);
                }
                break;
            }
        }
    }
}

fn check_expected(actual: &str, attr: &Attribute) -> bool {
    if let Some(meta_items) = attr.meta_item_list() {
        for meta_item in meta_items {
            if meta_item.check_name("expected") {
                let expected = meta_item.value_str().unwrap();

                if actual == &expected[..] {
                    println!("ok");
                } else {
                    println!("FAILED");
                    println!("\tActual value:\t{}", actual);
                    println!("\tExpected value:\t{}", expected);
                }

                return true;
            }
        }
    }

    false
}
