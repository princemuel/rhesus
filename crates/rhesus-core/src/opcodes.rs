//! Bytecode instruction set: typed `Instruction` enum, binary encoding,
//! decoding, and zero-allocation disassembly.
//!
//! Layout of one instruction: a 1-byte opcode tag followed by its operands
//! encoded big-endian.
//!
//! ```text
//! Instruction::Constant(65534)                     -> [ 0x00 | 0xFF 0xFE ]
//! Instruction::Closure { constant: 65534, free: 1 } -> [ 0x1B | 0xFF 0xFE | 0x01 ]
//! ```

use alloc::vec::Vec;
use core::fmt;
use core::ops::Deref;

use crate::errors::OpcodeError;

macro_rules! define_instructions {
    (
        $(
            $variant:ident $( ( $tuple_ty:ty ) )? $( { $($field:ident : $struct_ty:ty),+ } )?
        ),* $(,)?
    ) => {
        /// A single bytecode instruction paired with its statically-typed operands.
        #[repr(u8)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Instruction {
            $(
                $variant $( ( $tuple_ty ) )? $( { $($field : $struct_ty),+ } )?,
            )*
        }

        impl Instruction {
            /// Returns the disassembly name (e.g. `"OpConstant"`).
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(
                        define_instructions!(@pat $variant $( ( $tuple_ty ) )? $( { $($field),+ } )?)
                            => concat!("Op", stringify!($variant)),
                    )*
                }
            }
        }
    };

    (@pat $variant:ident) => { Self::$variant };
    (@pat $variant:ident ( $ty:ty )) => { Self::$variant(_) };
    (@pat $variant:ident { $($field:ident),+ }) => { Self::$variant { .. } };
}

define_instructions! {
    // Constants and stack
    Constant(u16),
    Add,
    Pop,
    Sub,
    Mul,
    Div,
    // Booleans and comparison
    True,
    False,
    Equal,
    NotEqual,
    GreaterThan,
    // Prefix operators
    Minus,
    Bang,
    // Control flow (operand: absolute instruction offset)
    JumpNotTruthy(u16),
    Jump(u16),
    Null,
    // Bindings
    GetGlobal(u16),
    SetGlobal(u16),
    // Data structures (operand: number of elements on the stack)
    Array(u16),
    Hash(u16),
    Index,
    // Functions (operand: number of arguments / slot index)
    Call(u8),
    ReturnValue,
    Return,
    GetLocal(u8),
    SetLocal(u8),
    GetBuiltin(u8),
    // Closures
    Closure { constant: u16, free: u8 },
    GetFree(u8),
    CurrentClosure,
}

impl Instruction {
    /// Returns the 1-byte opcode discriminant for this instruction (`0..=29`).
    #[must_use]
    pub const fn opcode(self) -> u8 {
        // SAFETY: `Instruction` is `#[repr(u8)]`, so its first byte is always
        // the initialized tag discriminant (guaranteed by the Rust Reference).
        unsafe { *(&raw const self).cast::<u8>() }
    }

    /// Appends this instruction's big-endian bytes to `out`.
    pub fn encode(self, out: &mut Vec<u8>) {
        let (buf, len) = self.to_array();
        out.extend_from_slice(&buf[..len]);
    }

    /// Decodes one instruction starting at the beginning of `bytes`.
    ///
    /// Returns `(instruction, bytes_consumed)` on success.
    ///
    /// # Errors
    ///
    /// - [`OpcodeError::Unknown`] if the leading opcode byte is unrecognized.
    /// - [`OpcodeError::Truncated`] if `bytes` ends before all operands are
    ///   present.
    pub const fn decode(bytes: &[u8]) -> Result<(Self, usize), OpcodeError> {
        const OP_CONSTANT: u8 = Instruction::Constant(0).opcode();
        const OP_ADD: u8 = Instruction::Add.opcode();
        const OP_POP: u8 = Instruction::Pop.opcode();
        const OP_SUB: u8 = Instruction::Sub.opcode();
        const OP_MUL: u8 = Instruction::Mul.opcode();
        const OP_DIV: u8 = Instruction::Div.opcode();
        const OP_TRUE: u8 = Instruction::True.opcode();
        const OP_FALSE: u8 = Instruction::False.opcode();
        const OP_EQUAL: u8 = Instruction::Equal.opcode();
        const OP_NOT_EQUAL: u8 = Instruction::NotEqual.opcode();
        const OP_GREATER_THAN: u8 = Instruction::GreaterThan.opcode();
        const OP_MINUS: u8 = Instruction::Minus.opcode();
        const OP_BANG: u8 = Instruction::Bang.opcode();
        const OP_JUMP_NOT_TRUTHY: u8 = Instruction::JumpNotTruthy(0).opcode();
        const OP_JUMP: u8 = Instruction::Jump(0).opcode();
        const OP_NULL: u8 = Instruction::Null.opcode();
        const OP_GET_GLOBAL: u8 = Instruction::GetGlobal(0).opcode();
        const OP_SET_GLOBAL: u8 = Instruction::SetGlobal(0).opcode();
        const OP_ARRAY: u8 = Instruction::Array(0).opcode();
        const OP_HASH: u8 = Instruction::Hash(0).opcode();
        const OP_INDEX: u8 = Instruction::Index.opcode();
        const OP_CALL: u8 = Instruction::Call(0).opcode();
        const OP_RETURN_VALUE: u8 = Instruction::ReturnValue.opcode();
        const OP_RETURN: u8 = Instruction::Return.opcode();
        const OP_GET_LOCAL: u8 = Instruction::GetLocal(0).opcode();
        const OP_SET_LOCAL: u8 = Instruction::SetLocal(0).opcode();
        const OP_GET_BUILTIN: u8 = Instruction::GetBuiltin(0).opcode();
        const OP_CLOSURE: u8 = Instruction::Closure { constant: 0, free: 0 }.opcode();
        const OP_GET_FREE: u8 = Instruction::GetFree(0).opcode();
        const OP_CURRENT_CLOSURE: u8 = Instruction::CurrentClosure.opcode();
        let Some((&op, rest)) = bytes.split_first() else {
            return Err(truncated("Instruction", 1, 0));
        };

        let instruction = match (op, rest) {
            // 0-operand instructions
            (OP_ADD, _) => Self::Add,
            (OP_POP, _) => Self::Pop,
            (OP_SUB, _) => Self::Sub,
            (OP_MUL, _) => Self::Mul,
            (OP_DIV, _) => Self::Div,
            (OP_TRUE, _) => Self::True,
            (OP_FALSE, _) => Self::False,
            (OP_EQUAL, _) => Self::Equal,
            (OP_NOT_EQUAL, _) => Self::NotEqual,
            (OP_GREATER_THAN, _) => Self::GreaterThan,
            (OP_MINUS, _) => Self::Minus,
            (OP_BANG, _) => Self::Bang,
            (OP_NULL, _) => Self::Null,
            (OP_INDEX, _) => Self::Index,
            (OP_RETURN_VALUE, _) => Self::ReturnValue,
            (OP_RETURN, _) => Self::Return,
            (OP_CURRENT_CLOSURE, _) => Self::CurrentClosure,

            // 1-byte operand instructions
            (OP_CALL, &[v, ..]) => Self::Call(v),
            (OP_GET_LOCAL, &[v, ..]) => Self::GetLocal(v),
            (OP_SET_LOCAL, &[v, ..]) => Self::SetLocal(v),
            (OP_GET_BUILTIN, &[v, ..]) => Self::GetBuiltin(v),
            (OP_GET_FREE, &[v, ..]) => Self::GetFree(v),

            // 2-byte big-endian operand instructions
            (OP_CONSTANT, &[hi, lo, ..]) => Self::Constant(u16::from_be_bytes([hi, lo])),
            (OP_JUMP_NOT_TRUTHY, &[hi, lo, ..]) => {
                Self::JumpNotTruthy(u16::from_be_bytes([hi, lo]))
            }
            (OP_JUMP, &[hi, lo, ..]) => Self::Jump(u16::from_be_bytes([hi, lo])),
            (OP_GET_GLOBAL, &[hi, lo, ..]) => Self::GetGlobal(u16::from_be_bytes([hi, lo])),
            (OP_SET_GLOBAL, &[hi, lo, ..]) => Self::SetGlobal(u16::from_be_bytes([hi, lo])),
            (OP_ARRAY, &[hi, lo, ..]) => Self::Array(u16::from_be_bytes([hi, lo])),
            (OP_HASH, &[hi, lo, ..]) => Self::Hash(u16::from_be_bytes([hi, lo])),

            // 3-byte operand instructions
            (OP_CLOSURE, &[hi, lo, free, ..]) => {
                Self::Closure { constant: u16::from_be_bytes([hi, lo]), free }
            }

            // Truncated valid opcodes
            (OP_CALL, _) => return Err(truncated(Self::Call(0).name(), 1, rest.len())),
            (OP_GET_LOCAL, _) => return Err(truncated(Self::GetLocal(0).name(), 1, rest.len())),
            (OP_SET_LOCAL, _) => return Err(truncated(Self::SetLocal(0).name(), 1, rest.len())),
            (OP_GET_BUILTIN, _) => {
                return Err(truncated(Self::GetBuiltin(0).name(), 1, rest.len()));
            }
            (OP_GET_FREE, _) => return Err(truncated(Self::GetFree(0).name(), 1, rest.len())),

            (OP_CONSTANT, _) => return Err(truncated(Self::Constant(0).name(), 2, rest.len())),
            (OP_JUMP_NOT_TRUTHY, _) => {
                return Err(truncated(Self::JumpNotTruthy(0).name(), 2, rest.len()));
            }
            (OP_JUMP, _) => return Err(truncated(Self::Jump(0).name(), 2, rest.len())),
            (OP_GET_GLOBAL, _) => return Err(truncated(Self::GetGlobal(0).name(), 2, rest.len())),

            (OP_SET_GLOBAL, _) => return Err(truncated(Self::SetGlobal(0).name(), 2, rest.len())),
            (OP_ARRAY, _) => return Err(truncated(Self::Array(0).name(), 2, rest.len())),
            (OP_HASH, _) => return Err(truncated(Self::Hash(0).name(), 2, rest.len())),
            (OP_CLOSURE, _) => {
                return Err(truncated(
                    Self::Closure { constant: 0, free: 0 }.name(),
                    3,
                    rest.len(),
                ));
            }
            _ => return Err(OpcodeError::Unknown(op)),
        };

        Ok((instruction, instruction.encoded_len()))
    }

    /// Encoded byte length of this instruction (1 opcode byte + operand bytes
    /// i.e 1..=4).
    #[must_use]
    const fn encoded_len(self) -> usize { self.to_array().1 }

    /// Encodes this instruction into a 4-byte stack buffer paired with its
    /// valid byte length (1..=4).
    #[must_use]
    const fn to_array(self) -> ([u8; 4], usize) {
        let op = self.opcode();
        match self {
            Self::Add
            | Self::Pop
            | Self::Sub
            | Self::Mul
            | Self::Div
            | Self::True
            | Self::False
            | Self::Equal
            | Self::NotEqual
            | Self::GreaterThan
            | Self::Minus
            | Self::Bang
            | Self::Null
            | Self::Index
            | Self::ReturnValue
            | Self::Return
            | Self::CurrentClosure => ([op, 0, 0, 0], 1),

            Self::Call(v)
            | Self::GetLocal(v)
            | Self::SetLocal(v)
            | Self::GetBuiltin(v)
            | Self::GetFree(v) => ([op, v, 0, 0], 2),

            Self::Constant(v)
            | Self::JumpNotTruthy(v)
            | Self::Jump(v)
            | Self::GetGlobal(v)
            | Self::SetGlobal(v)
            | Self::Array(v)
            | Self::Hash(v) => {
                let [hi, lo] = v.to_be_bytes();
                ([op, hi, lo, 0], 3)
            }

            Self::Closure { constant, free } => {
                let [hi, lo] = constant.to_be_bytes();
                ([op, hi, lo, free], 4)
            }
        }
    }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())?;
        match *self {
            Self::Constant(v)
            | Self::JumpNotTruthy(v)
            | Self::Jump(v)
            | Self::GetGlobal(v)
            | Self::SetGlobal(v)
            | Self::Array(v)
            | Self::Hash(v) => write!(f, " {v}"),
            Self::Call(v)
            | Self::GetLocal(v)
            | Self::SetLocal(v)
            | Self::GetBuiltin(v)
            | Self::GetFree(v) => write!(f, " {v}"),
            Self::Closure { constant, free } => write!(f, " {constant} {free}"),
            Self::Add
            | Self::Pop
            | Self::Sub
            | Self::Mul
            | Self::Div
            | Self::True
            | Self::False
            | Self::Equal
            | Self::NotEqual
            | Self::GreaterThan
            | Self::Minus
            | Self::Bang
            | Self::Null
            | Self::Index
            | Self::ReturnValue
            | Self::Return
            | Self::CurrentClosure => Ok(()),
        }
    }
}

const fn truncated(opcode: &'static str, expected: usize, got: usize) -> OpcodeError {
    OpcodeError::Truncated { opcode, expected, got }
}

impl From<Instruction> for Vec<u8> {
    fn from(ins: Instruction) -> Self {
        let (buf, len) = ins.to_array();
        buf[..len].to_vec()
    }
}

/// Reads a u8 from the front of ins; None if it is empty.
#[must_use]
pub const fn read_u8(ins: &[u8]) -> Option<u8> { ins.first().copied() }

/// Reads a big-endian u16 from the front of ins; None if shorter than 2 bytes.
#[must_use]
pub const fn read_u16(ins: &[u8]) -> Option<u16> {
    if let Some(&bytes) = ins.first_chunk() { Some(u16::from_be_bytes(bytes)) } else { None }
}

/// A bytecode stream sequence of encoded instructions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Instructions(pub Vec<u8>);
impl Instructions {
    /// Returns a zero-allocation [Disassembly] view over the bytecode.
    #[must_use]
    pub const fn disassemble(&self) -> Disassembly<'_> { Disassembly(&self.0) }
}

impl From<Vec<u8>> for Instructions {
    fn from(bytes: Vec<u8>) -> Self { Self(bytes) }
}

impl FromIterator<Instruction> for Instructions {
    fn from_iter<T: IntoIterator<Item = Instruction>>(iter: T) -> Self {
        let mut out = Vec::new();
        for ins in iter {
            ins.encode(&mut out);
        }
        Self(out)
    }
}

impl Deref for Instructions {
    type Target = [u8];

    fn deref(&self) -> &[u8] { &self.0 }
}
impl fmt::Display for Instructions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { self.disassemble().fmt(f) }
}

/// Zero-allocation Display wrapper that disassembles any borrowed bytecode
/// slice, one instruction per line: {offset:04} {instruction}.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Disassembly<'a>(pub &'a [u8]);

impl<'a> Disassembly<'a> {
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self { Self(bytes) }
}

impl<'a> From<&'a [u8]> for Disassembly<'a> {
    fn from(bytes: &'a [u8]) -> Self { Self(bytes) }
}

impl fmt::Display for Disassembly<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut offset = 0;
        while offset < self.0.len() {
            match Instruction::decode(&self.0[offset..]) {
                Ok((ins, read)) => {
                    writeln!(f, "{offset:04} {ins}")?;
                    offset += read;
                }
                Err(err) => {
                    writeln!(f, "ERROR: {err}")?;
                    break;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString as _;
    use alloc::vec;

    use super::*;
    const ALL_INSTRUCTIONS: &[Instruction] = &[
        Instruction::Constant(u16::MAX),
        Instruction::Add,
        Instruction::Pop,
        Instruction::Sub,
        Instruction::Mul,
        Instruction::Div,
        Instruction::True,
        Instruction::False,
        Instruction::Equal,
        Instruction::NotEqual,
        Instruction::GreaterThan,
        Instruction::Minus,
        Instruction::Bang,
        Instruction::JumpNotTruthy(u16::MAX),
        Instruction::Jump(u16::MAX),
        Instruction::Null,
        Instruction::GetGlobal(u16::MAX),
        Instruction::SetGlobal(u16::MAX),
        Instruction::Array(u16::MAX),
        Instruction::Hash(u16::MAX),
        Instruction::Index,
        Instruction::Call(u8::MAX),
        Instruction::ReturnValue,
        Instruction::Return,
        Instruction::GetLocal(u8::MAX),
        Instruction::SetLocal(u8::MAX),
        Instruction::GetBuiltin(u8::MAX),
        Instruction::Closure { constant: u16::MAX, free: u8::MAX },
        Instruction::GetFree(u8::MAX),
        Instruction::CurrentClosure,
    ];

    #[test]
    fn encode_writes_big_endian_operands() {
        assert_eq!(Vec::from(Instruction::Constant(0xfffe)), [0, 255, 254]);
        assert_eq!(Vec::from(Instruction::Add), [1]);
        assert_eq!(Vec::from(Instruction::GetLocal(255)), [24, 255]);
        assert_eq!(Vec::from(Instruction::Closure { constant: 0xfffe, free: 255 }), [
            27, 255, 254, 255
        ]);
    }

    #[test]
    fn encode_accepts_max_operand_values() {
        assert_eq!(Vec::from(Instruction::Constant(u16::MAX)), [0, 255, 255]);
        assert_eq!(Vec::from(Instruction::Call(u8::MAX)), [21, 255]);
    }

    #[test]
    fn encoded_len_matches_actual_encoded_bytes() {
        for &ins in ALL_INSTRUCTIONS {
            let bytes: Vec<_> = ins.into();
            assert_eq!(bytes.len(), ins.encoded_len(), "{ins:?}");
            assert_eq!(bytes[0], ins.opcode(), "{ins:?}");
        }
    }

    #[test]
    fn every_instruction_round_trips_encode_and_decode() {
        for (expected_opcode, &ins) in ALL_INSTRUCTIONS.iter().enumerate() {
            assert_eq!(usize::from(ins.opcode()), expected_opcode, "{ins:?}");
            let bytes: Vec<_> = ins.into();
            assert_eq!(Instruction::decode(&bytes), Ok((ins, bytes.len())), "{ins:?}");
        }
    }

    #[test]
    fn decode_ignores_trailing_bytes() {
        let mut stream = Instruction::Constant(7).into();
        Instruction::Add.encode(&mut stream);
        assert_eq!(Instruction::decode(&stream), Ok((Instruction::Constant(7), 3)));
    }

    #[test]
    fn decode_rejects_unknown_opcode() {
        let first_unused = u8::try_from(ALL_INSTRUCTIONS.len()).unwrap();
        assert_eq!(Instruction::decode(&[first_unused]), Err(OpcodeError::Unknown(first_unused)));
        assert_eq!(Instruction::decode(&[255]), Err(OpcodeError::Unknown(255)));
        assert_eq!(OpcodeError::Unknown(255).to_string(), "opcode 255 undefined");
    }

    #[test]
    fn decode_reports_truncated_input() {
        assert_eq!(
            Instruction::decode(&[]),
            Err(OpcodeError::Truncated { opcode: "Instruction", expected: 1, got: 0 })
        );
        assert_eq!(
            Instruction::decode(&[Instruction::Constant(0).opcode(), 1]),
            Err(OpcodeError::Truncated { opcode: "OpConstant", expected: 2, got: 1 })
        );
        // Closure has first operand (2 bytes) but is missing free (1
        // byte).
        assert_eq!(
            Instruction::decode(&[Instruction::Closure { constant: 0, free: 0 }.opcode(), 0, 1]),
            Err(OpcodeError::Truncated { opcode: "OpClosure", expected: 3, got: 2 })
        );
    }

    #[test]
    fn instructions_display_one_instruction_per_line() {
        let instructions: Instructions = [
            Instruction::Add,
            Instruction::GetLocal(1),
            Instruction::Constant(2),
            Instruction::Constant(u16::MAX),
            Instruction::Closure { constant: u16::MAX, free: u8::MAX },
        ]
        .into_iter()
        .collect();
        let expected = concat!(
            "0000 OpAdd\n",
            "0001 OpGetLocal 1\n",
            "0003 OpConstant 2\n",
            "0006 OpConstant 65535\n",
            "0009 OpClosure 65535 255\n",
        );
        assert_eq!(instructions.to_string(), expected);
    }

    #[test]
    fn displaying_nothing_prints_nothing() {
        assert_eq!(Instructions::default().to_string(), "");
        assert_eq!(Disassembly::new(&[]).to_string(), "");
    }

    #[test]
    fn disassembly_works_on_any_borrowed_slice() {
        let bytes = [Instruction::Add.opcode(), Instruction::Pop.opcode()];
        let expected = "0000 OpAdd\n0001 OpPop\n";
        assert_eq!(Disassembly::new(&bytes).to_string(), expected);
        assert_eq!(Disassembly::from(&bytes[..]).to_string(), expected);
        assert_eq!(Instructions::from(bytes.to_vec()).disassemble().to_string(), expected);
    }

    #[test]
    fn display_stops_at_an_unknown_opcode() {
        let mut bytes: Vec<_> = Instruction::Add.into();
        bytes.extend([255, Instruction::Add.opcode()]);
        assert_eq!(
            Instructions::from(bytes).to_string(),
            "0000 OpAdd\nERROR: opcode 255 undefined\n"
        );
    }

    #[test]
    fn display_stops_at_truncated_operands() {
        let bytes = vec![Instruction::Pop.opcode(), Instruction::Constant(0).opcode(), 0];
        assert_eq!(
            Instructions::from(bytes).to_string(),
            "0000 OpPop\nERROR: truncated OpConstant: needs 2 operand byte(s), got 1\n"
        );
    }

    #[test]
    fn read_u8_and_u16() {
        assert_eq!(read_u8(&[9, 1]), Some(9));
        assert_eq!(read_u8(&[]), None);
        assert_eq!(read_u16(&[1, 2, 3]), Some(0x0102));
        assert_eq!(read_u16(&[1]), None);
    }

    #[test]
    fn names_are_the_variant_names_prefixed_with_op() {
        assert_eq!(Instruction::Constant(0).name(), "OpConstant");
        assert_eq!(Instruction::NotEqual.name(), "OpNotEqual");
        assert_eq!(Instruction::CurrentClosure.name(), "OpCurrentClosure");
    }
}
