//! Bytecode instruction set: opcode definitions, encoding (`make`) and decoding
//! (`read_operands`, `Instructions`' `Display`).
//!
//! Layout of one instruction: a 1-byte opcode followed by its operands, each
//! encoded big-endian in the width its opcode declares.
//!
//! ```text
//! Constant 65534  ->  [ Constant | 0xFF 0xFE ]
//! Closure 65534 1 ->  [ Closure  | 0xFF 0xFE | 0x01 ]
//! ```

use alloc::vec::Vec;
use core::fmt;
use core::ops::Deref;

use crate::errors::OpcodeError;

/// How many bytes an operand occupies in the instruction stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    U8,
    U16,
}

impl Width {
    #[must_use]
    pub const fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
        }
    }
}

/// Declares the instruction set in ONE table, and generates from it:
/// the `Opcode` enum, `Opcode::ALL`, `Opcode::name` and
/// `Opcode::operand_widths`.
///
/// Each row is `Variant => [operand widths]`. Adding an opcode means adding one
/// row, so the enum, its byte value, its name and its operand layout can never
/// drift apart. The byte value is the row's position, so only ever append rows
/// (reordering changes the encoding of already-compiled bytecode).
macro_rules! opcodes {
    ($($variant:ident => [$($width:ident),*]),* $(,)?) => {
        #[repr(u8)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        pub enum Opcode {
            $($variant),*
        }

        impl Opcode {
            /// Every opcode, indexed by its byte value.
            pub const ALL: &'static [Self] = &[$(Self::$variant),*];

            /// The disassembly name, e.g. `OpConstant`.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => concat!("Op", stringify!($variant))),*
                }
            }

            /// The width of each operand in order. Operand-less opcodes are empty.
            #[must_use]
            pub const fn operand_widths(self) -> &'static [Width] {
                match self {
                    $(Self::$variant => &[$(Width::$width),*]),*
                }
            }
        }
    };
}

opcodes! {
    // Constants and stack
    Constant => [U16],
    Add => [],
    Pop => [],
    Sub => [],
    Mul => [],
    Div => [],
    // Booleans and comparison
    True => [],
    False => [],
    Equal => [],
    NotEqual => [],
    GreaterThan => [],
    // Prefix operators
    Minus => [],
    Bang => [],
    // Control flow (operand: absolute instruction offset)
    JumpNotTruthy => [U16],
    Jump => [U16],
    Null => [],
    // Bindings
    GetGlobal => [U16],
    SetGlobal => [U16],
    // Data structures (operand: number of elements on the stack)
    Array => [U16],
    Hash => [U16],
    Index => [],
    // Functions (operand: number of arguments)
    Call => [U8],
    ReturnValue => [],
    Return => [],
    GetLocal => [U8],
    SetLocal => [U8],
    GetBuiltin => [U8],
    // Closures (operands: constant index of the function, number of free variables)
    Closure => [U16, U8],
    GetFree => [U8],
    CurrentClosure => [],
}

impl Opcode {
    /// Total operand bytes that follow the opcode byte.
    #[must_use]
    pub fn operand_bytes(self) -> usize { self.operand_widths().iter().map(|w| w.bytes()).sum() }
}

impl From<Opcode> for u8 {
    fn from(op: Opcode) -> Self { op as Self }
}

impl TryFrom<u8> for Opcode {
    type Error = OpcodeError;

    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        Self::ALL.get(usize::from(byte)).copied().ok_or(OpcodeError::Unknown(byte))
    }
}

/// Encodes one instruction: the opcode byte followed by its big-endian
/// operands.
///
/// # Errors
///
/// [`OpcodeError::OperandCount`] if `operands` doesn't match what `op` takes,
/// [`OpcodeError::OperandOverflow`] if an operand doesn't fit its width (e.g.
/// the 65,536th constant, which a `u16` index can't address).
pub fn make(op: Opcode, operands: &[usize]) -> Result<Vec<u8>, OpcodeError> {
    let widths = op.operand_widths();
    if operands.len() != widths.len() {
        return Err(OpcodeError::OperandCount {
            opcode: op.name(),
            expected: widths.len(),
            got: operands.len(),
        });
    }

    let mut instruction = Vec::with_capacity(1 + op.operand_bytes());
    instruction.push(u8::from(op));

    for (&operand, &width) in operands.iter().zip(widths) {
        let overflow =
            || OpcodeError::OperandOverflow { opcode: op.name(), operand, bytes: width.bytes() };
        match width {
            Width::U8 => instruction.push(u8::try_from(operand).map_err(|_err| overflow())?),
            Width::U16 => instruction.extend_from_slice(
                &u16::try_from(operand).map_err(|_err| overflow())?.to_be_bytes(),
            ),
        }
    }

    Ok(instruction)
}

/// Decodes the operands of `op` from `ins`, which must start right AFTER the
/// opcode byte. Returns the operands and how many bytes they took, so the
/// caller can step to the next instruction.
///
/// # Errors
///
/// [`OpcodeError::Truncated`] if `ins` ends before all operands are read.
pub fn read_operands(op: Opcode, ins: &[u8]) -> Result<(Vec<usize>, usize), OpcodeError> {
    let truncated = || OpcodeError::Truncated {
        opcode: op.name(),
        expected: op.operand_bytes(),
        got: ins.len(),
    };

    let mut operands = Vec::with_capacity(op.operand_widths().len());
    let mut offset = 0;
    for &width in op.operand_widths() {
        let rest = ins.get(offset..).unwrap_or_default();

        let operand = match width {
            Width::U8 => read_u8(rest).map(usize::from),
            Width::U16 => read_u16(rest).map(usize::from),
        };
        operands.push(operand.ok_or_else(truncated)?);
        offset += width.bytes();
    }

    Ok((operands, offset))
}

/// Reads a `u8` from the front of `ins`; `None` if it is empty.
#[must_use]
pub const fn read_u8(ins: &[u8]) -> Option<u8> { ins.first().copied() }

/// Reads a big-endian `u16` from the front of `ins`; `None` if it is shorter
/// than 2 bytes.
#[must_use]
pub const fn read_u16(ins: &[u8]) -> Option<u16> {
    if let Some(&bytes) = ins.first_chunk() { Some(u16::from_be_bytes(bytes)) } else { None }
}

/// A sequence of encoded instructions.
///
/// `Display` disassembles it, one instruction per line: `{offset:04} {name}
/// {operands}`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Instructions(Vec<u8>);

impl From<Vec<u8>> for Instructions {
    fn from(bytes: Vec<u8>) -> Self { Self(bytes) }
}

impl Deref for Instructions {
    type Target = [u8];

    fn deref(&self) -> &[u8] { &self.0 }
}

impl fmt::Display for Instructions {
    /// Stops at the first undecodable byte with an `ERROR:` line instead of
    /// looping or panicking, so a bad stream is still readable in a test diff.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut offset = 0;
        while let Some((&byte, rest)) = self.0.get(offset..).and_then(<[u8]>::split_first) {
            let decoded = Opcode::try_from(byte).and_then(|op| {
                let (operands, read) = read_operands(op, rest)?;
                Ok((op, operands, read))
            });

            match decoded {
                Ok((op, operands, read)) => {
                    write!(f, "{offset:04} {}", op.name())?;
                    for operand in operands {
                        write!(f, " {operand}")?;
                    }
                    writeln!(f)?;
                    offset += 1 + read;
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

    fn ins(parts: &[Vec<u8>]) -> Instructions { Instructions::from(parts.concat()) }

    fn byte(op: Opcode) -> u8 { u8::from(op) }

    #[test]
    fn make_encodes_big_endian_operands() {
        let cases = [
            (Opcode::Constant, vec![0xfffe], vec![byte(Opcode::Constant), 255, 254]),
            (Opcode::Add, vec![], vec![byte(Opcode::Add)]),
            (Opcode::GetLocal, vec![255], vec![byte(Opcode::GetLocal), 255]),
            (Opcode::Closure, vec![0xfffe, 255], vec![byte(Opcode::Closure), 255, 254, 255]),
        ];
        for (op, operands, expected) in cases {
            assert_eq!(make(op, &operands), Ok(expected), "{op:?} {operands:?}");
        }
    }

    #[test]
    fn make_accepts_the_largest_operand_of_each_width() {
        assert_eq!(make(Opcode::Constant, &[0xffff]), Ok(vec![byte(Opcode::Constant), 255, 255]));
        assert_eq!(make(Opcode::Call, &[255]), Ok(vec![byte(Opcode::Call), 255]));
    }

    #[test]
    fn make_rejects_the_wrong_number_of_operands() {
        assert_eq!(
            make(Opcode::Add, &[1]),
            Err(OpcodeError::OperandCount { opcode: "OpAdd", expected: 0, got: 1 })
        );
        assert_eq!(
            make(Opcode::Constant, &[]),
            Err(OpcodeError::OperandCount { opcode: "OpConstant", expected: 1, got: 0 })
        );
        assert_eq!(
            make(Opcode::Closure, &[1]),
            Err(OpcodeError::OperandCount { opcode: "OpClosure", expected: 2, got: 1 })
        );
    }

    #[test]
    fn make_rejects_operands_that_do_not_fit() {
        assert_eq!(
            make(Opcode::Constant, &[0x0001_0000]),
            Err(OpcodeError::OperandOverflow {
                opcode: "OpConstant",
                operand: 0x0001_0000,
                bytes: 2
            })
        );
        assert_eq!(
            make(Opcode::GetLocal, &[256]),
            Err(OpcodeError::OperandOverflow { opcode: "OpGetLocal", operand: 256, bytes: 1 })
        );
        // The second operand of Closure is the narrow one.
        assert_eq!(
            make(Opcode::Closure, &[0, 256]),
            Err(OpcodeError::OperandOverflow { opcode: "OpClosure", operand: 256, bytes: 1 })
        );
    }

    #[test]
    fn instructions_display_one_instruction_per_line() {
        let instructions = ins(&[
            make(Opcode::Add, &[]).unwrap(),
            make(Opcode::GetLocal, &[1]).unwrap(),
            make(Opcode::Constant, &[2]).unwrap(),
            make(Opcode::Constant, &[0xffff]).unwrap(),
            make(Opcode::Closure, &[0xffff, 255]).unwrap(),
        ]);

        let expected = "\
0000 OpAdd
0001 OpGetLocal 1
0003 OpConstant 2
0006 OpConstant 65535
0009 OpClosure 65535 255
";
        assert_eq!(instructions.to_string(), expected);
    }

    #[test]
    fn displaying_nothing_prints_nothing() {
        assert_eq!(Instructions::default().to_string(), "");
    }

    #[test]
    fn display_stops_at_an_unknown_opcode() {
        let mut bytes = make(Opcode::Add, &[]).unwrap();
        bytes.extend([255, byte(Opcode::Add)]);

        assert_eq!(
            Instructions::from(bytes).to_string(),
            "0000 OpAdd\nERROR: opcode 255 undefined\n"
        );
    }

    #[test]
    fn display_stops_at_truncated_operands() {
        let bytes = vec![byte(Opcode::Pop), byte(Opcode::Constant), 0];

        assert_eq!(
            Instructions::from(bytes).to_string(),
            "0000 OpPop\nERROR: truncated OpConstant: needs 2 operand byte(s), got 1\n"
        );
    }

    #[test]
    fn read_operands_round_trips_make() {
        let cases = [
            (Opcode::Constant, vec![0xffff], 2),
            (Opcode::GetLocal, vec![255], 1),
            (Opcode::Closure, vec![0xffff, 255], 3),
            (Opcode::Add, vec![], 0),
        ];
        for (op, operands, bytes_read) in cases {
            let instruction = make(op, &operands).unwrap();
            let read = read_operands(op, &instruction[1..]);
            assert_eq!(read, Ok((operands, bytes_read)), "{op:?}");
        }
    }

    #[test]
    fn read_operands_reports_truncated_input() {
        assert_eq!(
            read_operands(Opcode::Constant, &[1]),
            Err(OpcodeError::Truncated { opcode: "OpConstant", expected: 2, got: 1 })
        );
        // The first operand is present, the second is not.
        assert_eq!(
            read_operands(Opcode::Closure, &[0, 1]),
            Err(OpcodeError::Truncated { opcode: "OpClosure", expected: 3, got: 2 })
        );
    }

    #[test]
    fn read_operands_ignores_bytes_after_the_instruction() {
        let stream = [0, 7, byte(Opcode::Add)];
        assert_eq!(read_operands(Opcode::Constant, &stream), Ok((vec![7], 2)));
    }

    #[test]
    fn read_u8_and_u16() {
        assert_eq!(read_u8(&[9, 1]), Some(9));
        assert_eq!(read_u8(&[]), None);
        assert_eq!(read_u16(&[1, 2, 3]), Some(0x0102));
        assert_eq!(read_u16(&[1]), None);
    }

    #[test]
    fn every_opcode_survives_a_byte_round_trip() {
        for (index, &op) in Opcode::ALL.iter().enumerate() {
            assert_eq!(usize::from(byte(op)), index, "{op:?} is out of order in ALL");
            assert_eq!(Opcode::try_from(byte(op)), Ok(op));
        }
    }

    #[test]
    fn unknown_bytes_are_rejected() {
        let first_unused = u8::try_from(Opcode::ALL.len()).unwrap();
        assert_eq!(Opcode::try_from(first_unused), Err(OpcodeError::Unknown(first_unused)));
        assert_eq!(Opcode::try_from(255), Err(OpcodeError::Unknown(255)));
        assert_eq!(OpcodeError::Unknown(255).to_string(), "opcode 255 undefined");
    }

    #[test]
    fn every_opcode_round_trips_at_its_widest_operands() {
        for &op in Opcode::ALL {
            let operands: Vec<usize> = op
                .operand_widths()
                .iter()
                .map(|w| match w {
                    Width::U8 => usize::from(u8::MAX),
                    Width::U16 => usize::from(u16::MAX),
                })
                .collect();

            let instruction = make(op, &operands).unwrap();
            assert_eq!(instruction.len(), 1 + op.operand_bytes(), "{op:?}");
            assert_eq!(instruction.first(), Some(&byte(op)), "{op:?}");
            assert_eq!(
                read_operands(op, &instruction[1..]),
                Ok((operands, op.operand_bytes())),
                "{op:?}"
            );
        }
    }

    #[test]
    fn names_are_the_variant_names_prefixed_with_op() {
        assert_eq!(Opcode::Constant.name(), "OpConstant");
        assert_eq!(Opcode::NotEqual.name(), "OpNotEqual");
        assert_eq!(Opcode::CurrentClosure.name(), "OpCurrentClosure");
    }
}
