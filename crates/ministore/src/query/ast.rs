#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Pred(Predicate),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    Has { field: String },

    // path special
    PathGlob { pattern: String },

    // keyword
    Keyword { field: String, pattern: String, kind: KeywordPatternKind },

    // text / FTS
    Text { field: Option<String>, fts: String }, // field None means "all text fields"

    // number
    NumberCmp { field: String, op: CmpOp, value: f64 },
    NumberRange { field: String, lo: f64, hi: f64 },

    // date
    DateCmpAbs { field: String, op: CmpOp, epoch_ms: i64 },
    DateRangeAbs { field: String, lo_ms: i64, hi_ms: i64 },
    DateCmpRel { field: String, op: CmpOp, amount: i64, unit: RelUnit }, // interpreted later

    // bool
    Bool { field: String, value: bool },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RelUnit {
    H, // hours
    D, // days
    W, // weeks
    M, // months
    Y, // years
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum KeywordPatternKind {
    Exact,
    Prefix,
    Contains,
    Glob, // includes '?' mixed with '*'
}
