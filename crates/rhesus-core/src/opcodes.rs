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

            /// One instance per variant with all operands zeroed, in opcode order:
            /// `PROTOTYPES[op]` is the instruction whose tag byte is `op`.
            const PROTOTYPES: &'static [Self] = &[
                $( define_instructions!(@proto $variant $( ( $tuple_ty ) )? $( { $($field : $struct_ty),+ } )?) ),*
            ];
        }
    };

    (@pat $variant:ident) => { Self::$variant };
    (@pat $variant:ident ( $ty:ty )) => { Self::$variant(_) };
    (@pat $variant:ident { $($field:ident),+ }) => { Self::$variant { .. } };
    (@proto $variant:ident) => { Instruction::$variant };
    (@proto $variant:ident ( $ty:ty )) => { Instruction::$variant(0) };
    (@proto $variant:ident { $($field:ident : $ty:ty),+ }) => { Instruction::$variant { $($field: 0),+ } };
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
        // SAFETY:
        // `Instruction` is `#[repr(u8)]`, so its first byte is always
        // the initialized tag discriminant as stated in the Rust Reference ;)
        unsafe { *(&raw const self).cast() }
    }

    /// Appends this instruction's big-endian bytes to `out`.
    pub fn encode(self, out: &mut Vec<u8>) {
        let (buf, len) = self.as_array();
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
        let Some((&op, rest)) = bytes.split_first() else {
            return Err(truncated("Instruction", 1, 0));
        };

        let index = op as usize; // `usize::from` isn't usable here; this widening is lossless
        if index >= Self::PROTOTYPES.len() {
            return Err(OpcodeError::Unknown(op));
        }
        let proto = Self::PROTOTYPES[index];
        let operands = proto.encoded_len() - 1;

        let instruction = match (proto, rest) {
            // 1-byte operand
            (Self::Call(_), &[v, ..]) => Self::Call(v),
            (Self::GetLocal(_), &[v, ..]) => Self::GetLocal(v),
            (Self::SetLocal(_), &[v, ..]) => Self::SetLocal(v),
            (Self::GetBuiltin(_), &[v, ..]) => Self::GetBuiltin(v),
            (Self::GetFree(_), &[v, ..]) => Self::GetFree(v),

            // 2-byte big-endian operand
            (Self::Constant(_), &[hi, lo, ..]) => Self::Constant(u16::from_be_bytes([hi, lo])),
            (Self::JumpNotTruthy(_), &[hi, lo, ..]) => {
                Self::JumpNotTruthy(u16::from_be_bytes([hi, lo]))
            }
            (Self::Jump(_), &[hi, lo, ..]) => Self::Jump(u16::from_be_bytes([hi, lo])),
            (Self::GetGlobal(_), &[hi, lo, ..]) => Self::GetGlobal(u16::from_be_bytes([hi, lo])),
            (Self::SetGlobal(_), &[hi, lo, ..]) => Self::SetGlobal(u16::from_be_bytes([hi, lo])),
            (Self::Array(_), &[hi, lo, ..]) => Self::Array(u16::from_be_bytes([hi, lo])),
            (Self::Hash(_), &[hi, lo, ..]) => Self::Hash(u16::from_be_bytes([hi, lo])),

            // 3-byte operand
            (Self::Closure { .. }, &[hi, lo, free, ..]) => {
                Self::Closure { constant: u16::from_be_bytes([hi, lo]), free }
            }

            // No operands: the prototype already is the instruction.
            (unit, _) if operands == 0 => unit,

            // An operand-carrying opcode with too few bytes left.
            (short, _) => return Err(truncated(short.name(), operands, rest.len())),
        };

        Ok((instruction, instruction.encoded_len()))
    }

    /// Encoded byte length of this instruction (1 opcode byte + operand bytes
    /// i.e 1..=4).
    #[must_use]
    const fn encoded_len(self) -> usize { self.as_array().1 }

    /// Encodes this instruction into a 4-byte stack buffer paired with its
    /// valid byte length (1..=4).
    #[must_use]
    const fn as_array(self) -> ([u8; 4], usize) {
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
        f.write_str(self.name())?;

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
        let (buf, len) = ins.as_array();
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
    fn prototypes_are_indexed_by_opcode() {
        assert_eq!(
            ALL_INSTRUCTIONS.len(),
            Instruction::PROTOTYPES.len(),
            "ALL_INSTRUCTIONS misses a variant"
        );
        for (index, proto) in Instruction::PROTOTYPES.iter().enumerate() {
            assert_eq!(usize::from(proto.opcode()), index, "{proto}");
        }
        for (index, ins) in ALL_INSTRUCTIONS.iter().enumerate() {
            assert_eq!(usize::from(ins.opcode()), index, "{ins}");
        }
    }

    #[test]
    fn every_instruction_round_trips_and_reports_truncation() {
        for &ins in ALL_INSTRUCTIONS {
            let mut out = Vec::new();
            ins.encode(&mut out);
            assert_eq!(Instruction::decode(&out), Ok((ins, out.len())));

            if out.len() > 1 {
                let short = &out[..out.len() - 1];
                assert_eq!(
                    Instruction::decode(short),
                    Err(OpcodeError::Truncated {
                        opcode: ins.name(),
                        expected: out.len() - 1,
                        got: out.len() - 2,
                    })
                );
            }
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
