//! The 11 operator routings of the FM Synth, as a table.
//!
//! Operators are `A` (index 0) to `D` (index 3). An arrow `X→Y` means X phase-modulates Y;
//! carriers (marked `*`) are summed to the output. Every modulator has a higher index than
//! the operators it modulates, so one pass from D down to A renders any algorithm.
//!
//! ```text
//!  1  D→C→B→A          2  (C+D)→B→A        3  (D→C)+B→A        4  (B+C+D)→A
//!     D                   C   D               D                   B  C  D
//!     |                    \ /                |                    \ | /
//!     C                     B                 C   B                  A*
//!     |                     |                  \ /
//!     B                     A*                  A*
//!     |
//!     A*
//!
//!  5  D→(B,C)→A        6  D→C→B, A          7  B→A, D→C          8  D→C→(A,B)
//!       D                 D                   B   D                  D
//!      / \                |                   |   |                  |
//!     B   C               C                   A*  C*                 C
//!      \ /                |                                         / \
//!       A*                B*  A*                                   A*  B*
//!
//!  9  D→C, A, B        10  D→(A,B,C)        11  A+B+C+D
//!     D                       D
//!     |                     / | \
//!     C*  A*  B*           A* B* C*           A*  B*  C*  D*
//! ```
//!
//! The first (a stack, every operator modulating the next) and the last (four parallel
//! sines, additive) match the order of Live's Operator, which the Ableton importer relies on.

/// Number of operators.
pub const OPS: usize = 4;

/// Routing of one algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Algorithm {
    /// Per operator, the bit mask of its modulators (bit `j` = operator `j`).
    pub modulators: [u8; OPS],
    /// Bit mask of the carriers (operators heard at the output).
    pub carriers: u8,
    /// Choice label: the routing in arrow notation.
    pub label: &'static str,
}

const A: u8 = 1;
const B: u8 = 2;
const C: u8 = 4;
const D: u8 = 8;

/// The algorithms, in `Algorithm` param order.
pub const ALGORITHMS: [Algorithm; 11] = [
    Algorithm {
        modulators: [B, C, D, 0],
        carriers: A,
        label: "D→C→B→A",
    },
    Algorithm {
        modulators: [B, C | D, 0, 0],
        carriers: A,
        label: "(C+D)→B→A",
    },
    Algorithm {
        modulators: [B | C, 0, D, 0],
        carriers: A,
        label: "(D→C)+B→A",
    },
    Algorithm {
        modulators: [B | C | D, 0, 0, 0],
        carriers: A,
        label: "(B+C+D)→A",
    },
    Algorithm {
        modulators: [B | C, D, D, 0],
        carriers: A,
        label: "D→(B,C)→A",
    },
    Algorithm {
        modulators: [0, C, D, 0],
        carriers: A | B,
        label: "D→C→B, A",
    },
    Algorithm {
        modulators: [B, 0, D, 0],
        carriers: A | C,
        label: "B→A, D→C",
    },
    Algorithm {
        modulators: [C, C, D, 0],
        carriers: A | B,
        label: "D→C→(A,B)",
    },
    Algorithm {
        modulators: [0, 0, D, 0],
        carriers: A | B | C,
        label: "D→C, A, B",
    },
    Algorithm {
        modulators: [D, D, D, 0],
        carriers: A | B | C,
        label: "D→(A,B,C)",
    },
    Algorithm {
        modulators: [0, 0, 0, 0],
        carriers: A | B | C | D,
        label: "A+B+C+D",
    },
];

/// The algorithm labels (the `Algorithm` param's choices).
pub fn labels() -> [&'static str; ALGORITHMS.len()] {
    ALGORITHMS.map(|a| a.label)
}

impl Algorithm {
    /// Is operator `op` heard at the output?
    #[inline]
    pub fn is_carrier(&self, op: usize) -> bool {
        self.carriers & (1 << op) != 0
    }

    /// Operators that `op` modulates (bit mask).
    pub fn targets(&self, op: usize) -> u8 {
        let mut out = 0;
        for (i, m) in self.modulators.iter().enumerate() {
            if m & (1 << op) != 0 {
                out |= 1 << i;
            }
        }
        out
    }

    /// The routing for the layout widget.
    pub fn routing(&self) -> ether_core::protocol::layout::FmRouting {
        ether_core::protocol::layout::FmRouting {
            modulators: self.modulators.to_vec(),
            carriers: self.carriers,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operator_is_heard_or_modulates_a_lower_one() {
        for (i, a) in ALGORITHMS.iter().enumerate() {
            assert!(a.carriers != 0, "algorithm {}: no carrier", i + 1);
            for op in 0..OPS {
                // Modulators come from higher operators only (one pass D → A renders it).
                assert_eq!(
                    a.modulators[op] & ((1 << (op + 1)) - 1),
                    0,
                    "algorithm {}: op {op} modulated by itself or a lower op",
                    i + 1
                );
                assert!(
                    a.is_carrier(op) || a.targets(op) != 0,
                    "algorithm {}: op {op} is unused",
                    i + 1
                );
                // A carrier is not also a modulator (the diagrams are trees into the output).
                assert!(
                    !(a.is_carrier(op) && a.targets(op) != 0),
                    "algorithm {}: op {op} is both",
                    i + 1
                );
            }
        }
    }

    #[test]
    fn algorithms_are_distinct_and_labelled() {
        for (i, a) in ALGORITHMS.iter().enumerate() {
            for b in &ALGORITHMS[i + 1..] {
                assert!(a.modulators != b.modulators || a.carriers != b.carriers);
                assert_ne!(a.label, b.label);
            }
        }
        // Live's order at the ends (the importer maps by index).
        assert_eq!(ALGORITHMS[0].carriers, A);
        assert_eq!(ALGORITHMS[0].modulators, [B, C, D, 0]);
        assert_eq!(ALGORITHMS[10].carriers, 0b1111);
    }
}
