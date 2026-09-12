use crate::Point2;

pub const MAX_MATERIAL_PARAMETERS: usize = 8;
pub const MAX_FORMULA_BYTES: usize = 256;
const MAX_FORMULA_OPS: usize = 128;
const MAX_PARSE_DEPTH: usize = 32;
const MAX_EVAL_STACK: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum MaterialError {
    EmptyFormula,
    FormulaTooLong,
    UnexpectedToken(usize),
    UnknownFunction(String),
    InvalidArgumentCount(String),
    TooComplex,
    MissingParameter(String),
    InvalidValue,
}

impl std::fmt::Display for MaterialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyFormula => write!(f, "formula is empty"),
            Self::FormulaTooLong => write!(f, "formula exceeds {MAX_FORMULA_BYTES} bytes"),
            Self::UnexpectedToken(offset) => write!(f, "unexpected token at byte {offset}"),
            Self::UnknownFunction(name) => write!(f, "unknown function `{name}`"),
            Self::InvalidArgumentCount(name) => {
                write!(f, "function `{name}` has the wrong number of arguments")
            }
            Self::TooComplex => write!(f, "formula is too complex"),
            Self::MissingParameter(name) => write!(f, "missing parameter `{name}`"),
            Self::InvalidValue => write!(f, "formula produced an invalid value"),
        }
    }
}

impl std::error::Error for MaterialError {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialCoordinates {
    pub x: f64,
    pub y: f64,
    pub r: f64,
    pub theta: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialFrameAttachment {
    World,
    FollowRegion,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialFrame {
    pub origin: Point2,
    pub angle_radians: f64,
    pub attachment: MaterialFrameAttachment,
}

impl MaterialFrame {
    pub const fn world() -> Self {
        Self {
            origin: Point2::new(0.0, 0.0),
            angle_radians: 0.0,
            attachment: MaterialFrameAttachment::World,
        }
    }

    pub fn valid(self) -> bool {
        self.origin.finite() && self.angle_radians.is_finite()
    }

    pub fn coordinates(self, point: Point2) -> MaterialCoordinates {
        let relative = point - self.origin;
        let (sin, cos) = self.angle_radians.sin_cos();
        let x = cos * relative.x + sin * relative.y;
        let y = -sin * relative.x + cos * relative.y;
        MaterialCoordinates {
            x,
            y,
            r: x.hypot(y),
            theta: y.atan2(x),
        }
    }
}

impl Default for MaterialFrame {
    fn default() -> Self {
        Self::world()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterialParameter {
    pub name: String,
    pub value: f64,
}

impl MaterialParameter {
    pub fn valid(&self) -> bool {
        valid_identifier(&self.name) && !reserved_identifier(&self.name) && self.value.is_finite()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScalarField {
    Constant(f64),
    Formula(MaterialFormula),
}

impl ScalarField {
    pub const fn constant(value: f64) -> Self {
        Self::Constant(value)
    }

    pub fn formula(source: impl Into<String>) -> Result<Self, MaterialError> {
        Ok(Self::Formula(MaterialFormula::parse(source)?))
    }

    pub fn source(&self) -> Option<&str> {
        match self {
            Self::Constant(_) => None,
            Self::Formula(formula) => Some(formula.source()),
        }
    }

    pub fn constant_value(&self) -> Option<f64> {
        match self {
            Self::Constant(value) => Some(*value),
            Self::Formula(_) => None,
        }
    }

    pub fn evaluate(
        &self,
        coordinates: MaterialCoordinates,
        parameters: &[MaterialParameter],
    ) -> Result<f64, MaterialError> {
        let value = match self {
            Self::Constant(value) => *value,
            Self::Formula(formula) => formula.evaluate(coordinates, parameters)?,
        };
        value
            .is_finite()
            .then_some(value)
            .ok_or(MaterialError::InvalidValue)
    }

    pub fn parameter_names(&self) -> impl Iterator<Item = &str> {
        self.formula_parameters().iter().map(String::as_str)
    }

    pub fn rename_parameter(&self, old: &str, new: &str) -> Result<Self, MaterialError> {
        match self {
            Self::Constant(value) => Ok(Self::Constant(*value)),
            Self::Formula(formula) => Self::formula(replace_identifier(formula.source(), old, new)),
        }
    }

    fn formula_parameters(&self) -> &[String] {
        match self {
            Self::Constant(_) => &[],
            Self::Formula(formula) => &formula.parameters,
        }
    }
}

impl From<f64> for ScalarField {
    fn from(value: f64) -> Self {
        Self::Constant(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EvaluatedMaterial {
    pub mass_density: f64,
    pub stiffness: f64,
    pub damping: f64,
}

impl EvaluatedMaterial {
    pub fn valid(self) -> bool {
        self.mass_density.is_finite()
            && self.mass_density > 0.0
            && self.stiffness.is_finite()
            && self.stiffness > 0.0
            && self.damping.is_finite()
            && self.damping >= 0.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaterialFormula {
    source: String,
    program: Vec<Instruction>,
    parameters: Vec<String>,
}

impl MaterialFormula {
    pub fn parse(source: impl Into<String>) -> Result<Self, MaterialError> {
        let source = source.into();
        if source.trim().is_empty() {
            return Err(MaterialError::EmptyFormula);
        }
        if source.len() > MAX_FORMULA_BYTES {
            return Err(MaterialError::FormulaTooLong);
        }
        let mut parser = Parser::new(&source);
        parser.parse_expression(0)?;
        parser.skip_space();
        if parser.offset != source.len() {
            return Err(MaterialError::UnexpectedToken(parser.offset));
        }
        if parser.program.len() > MAX_FORMULA_OPS {
            return Err(MaterialError::TooComplex);
        }
        let mut parameters = Vec::new();
        for instruction in &parser.program {
            if let Instruction::Parameter(name) = instruction
                && !parameters.contains(name)
            {
                parameters.push(name.clone());
            }
        }
        let program = parser.program;
        Ok(Self {
            source,
            program,
            parameters,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn evaluate(
        &self,
        coordinates: MaterialCoordinates,
        parameters: &[MaterialParameter],
    ) -> Result<f64, MaterialError> {
        let mut stack = [0.0; MAX_EVAL_STACK];
        let mut stack_len = 0;
        for instruction in &self.program {
            let value = match instruction {
                Instruction::Constant(value) => Some(*value),
                Instruction::X => Some(coordinates.x),
                Instruction::Y => Some(coordinates.y),
                Instruction::R => Some(coordinates.r),
                Instruction::Theta => Some(coordinates.theta),
                Instruction::Parameter(name) => Some(
                    parameters
                        .iter()
                        .find(|parameter| parameter.name == *name)
                        .map(|parameter| parameter.value)
                        .ok_or_else(|| MaterialError::MissingParameter(name.clone()))?,
                ),
                Instruction::Neg => {
                    unary(&mut stack, &mut stack_len, |value| -value)?;
                    None
                }
                Instruction::Add => {
                    binary(&mut stack, &mut stack_len, |a, b| a + b)?;
                    None
                }
                Instruction::Sub => {
                    binary(&mut stack, &mut stack_len, |a, b| a - b)?;
                    None
                }
                Instruction::Mul => {
                    binary(&mut stack, &mut stack_len, |a, b| a * b)?;
                    None
                }
                Instruction::Div => {
                    binary(&mut stack, &mut stack_len, |a, b| a / b)?;
                    None
                }
                Instruction::Pow => {
                    binary(&mut stack, &mut stack_len, f64::powf)?;
                    None
                }
                Instruction::Function(function) => {
                    function.evaluate(&mut stack, &mut stack_len)?;
                    None
                }
            };
            if let Some(value) = value {
                push_value(&mut stack, &mut stack_len, value)?;
            }
        }
        if stack_len != 1 || !stack[0].is_finite() {
            return Err(MaterialError::InvalidValue);
        }
        Ok(stack[0])
    }
}

fn push_value(
    stack: &mut [f64; MAX_EVAL_STACK],
    len: &mut usize,
    value: f64,
) -> Result<(), MaterialError> {
    if !value.is_finite() {
        return Err(MaterialError::InvalidValue);
    }
    if *len == stack.len() {
        return Err(MaterialError::TooComplex);
    }
    stack[*len] = value;
    *len += 1;
    Ok(())
}

fn pop_value(stack: &[f64; MAX_EVAL_STACK], len: &mut usize) -> Result<f64, MaterialError> {
    if *len == 0 {
        return Err(MaterialError::InvalidValue);
    }
    *len -= 1;
    Ok(stack[*len])
}

fn unary(
    stack: &mut [f64; MAX_EVAL_STACK],
    len: &mut usize,
    function: impl FnOnce(f64) -> f64,
) -> Result<(), MaterialError> {
    let value = pop_value(stack, len)?;
    push_value(stack, len, function(value))
}

fn binary(
    stack: &mut [f64; MAX_EVAL_STACK],
    len: &mut usize,
    function: impl FnOnce(f64, f64) -> f64,
) -> Result<(), MaterialError> {
    let right = pop_value(stack, len)?;
    let left = pop_value(stack, len)?;
    push_value(stack, len, function(left, right))
}

#[derive(Clone, Debug, PartialEq)]
enum Instruction {
    Constant(f64),
    X,
    Y,
    R,
    Theta,
    Parameter(String),
    Neg,
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    Function(Function),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Function {
    Sqrt,
    Abs,
    Sin,
    Cos,
    Tan,
    Exp,
    Log,
    Min,
    Max,
    Clamp,
    Smoothstep,
}

impl Function {
    fn named(name: &str) -> Result<(Self, usize), MaterialError> {
        Ok(match name {
            "sqrt" => (Self::Sqrt, 1),
            "abs" => (Self::Abs, 1),
            "sin" => (Self::Sin, 1),
            "cos" => (Self::Cos, 1),
            "tan" => (Self::Tan, 1),
            "exp" => (Self::Exp, 1),
            "log" => (Self::Log, 1),
            "min" => (Self::Min, 2),
            "max" => (Self::Max, 2),
            "clamp" => (Self::Clamp, 3),
            "smoothstep" => (Self::Smoothstep, 3),
            _ => return Err(MaterialError::UnknownFunction(name.into())),
        })
    }

    fn evaluate(
        self,
        stack: &mut [f64; MAX_EVAL_STACK],
        len: &mut usize,
    ) -> Result<(), MaterialError> {
        match self {
            Self::Sqrt => unary(stack, len, f64::sqrt),
            Self::Abs => unary(stack, len, f64::abs),
            Self::Sin => unary(stack, len, f64::sin),
            Self::Cos => unary(stack, len, f64::cos),
            Self::Tan => unary(stack, len, f64::tan),
            Self::Exp => unary(stack, len, f64::exp),
            Self::Log => unary(stack, len, f64::ln),
            Self::Min => binary(stack, len, f64::min),
            Self::Max => binary(stack, len, f64::max),
            Self::Clamp => {
                let value = pop_value(stack, len)?;
                let maximum = pop_value(stack, len)?;
                let minimum = pop_value(stack, len)?;
                if minimum > maximum {
                    return Err(MaterialError::InvalidValue);
                }
                push_value(stack, len, value.clamp(minimum, maximum))
            }
            Self::Smoothstep => {
                let value = pop_value(stack, len)?;
                let edge1 = pop_value(stack, len)?;
                let edge0 = pop_value(stack, len)?;
                if edge0 >= edge1 {
                    return Err(MaterialError::InvalidValue);
                }
                let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
                push_value(stack, len, t * t * (3.0 - 2.0 * t))
            }
        }
    }
}

struct Parser<'a> {
    source: &'a str,
    offset: usize,
    depth: usize,
    program: Vec<Instruction>,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            offset: 0,
            depth: 0,
            program: Vec::new(),
        }
    }

    fn parse_expression(&mut self, minimum_precedence: u8) -> Result<(), MaterialError> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            return Err(MaterialError::TooComplex);
        }
        self.skip_space();
        if self.consume('-') {
            self.parse_expression(3)?;
            self.push(Instruction::Neg)?;
        } else if self.consume('+') {
            self.parse_expression(3)?;
        } else {
            self.parse_primary()?;
        }
        loop {
            self.skip_space();
            let Some((operator, precedence, right_associative)) = self.peek_operator() else {
                break;
            };
            if precedence < minimum_precedence {
                break;
            }
            self.offset += 1;
            self.parse_expression(if right_associative {
                precedence
            } else {
                precedence + 1
            })?;
            self.push(operator)?;
        }
        self.depth -= 1;
        Ok(())
    }

    fn parse_primary(&mut self) -> Result<(), MaterialError> {
        self.skip_space();
        if self.consume('(') {
            self.parse_expression(0)?;
            self.skip_space();
            if !self.consume(')') {
                return Err(MaterialError::UnexpectedToken(self.offset));
            }
            return Ok(());
        }
        if self
            .peek()
            .is_some_and(|character| character.is_ascii_digit() || character == '.')
        {
            return self.parse_number();
        }
        let start = self.offset;
        let name = self.parse_identifier()?;
        self.skip_space();
        if self.consume('(') {
            let (function, expected) = Function::named(name)?;
            let mut arguments = 0;
            self.skip_space();
            if !self.consume(')') {
                loop {
                    self.parse_expression(0)?;
                    arguments += 1;
                    self.skip_space();
                    if self.consume(')') {
                        break;
                    }
                    if !self.consume(',') {
                        return Err(MaterialError::UnexpectedToken(self.offset));
                    }
                }
            }
            if arguments != expected {
                return Err(MaterialError::InvalidArgumentCount(name.into()));
            }
            return self.push(Instruction::Function(function));
        }
        let instruction = match name {
            "x" => Instruction::X,
            "y" => Instruction::Y,
            "r" => Instruction::R,
            "theta" => Instruction::Theta,
            "pi" => Instruction::Constant(std::f64::consts::PI),
            "e" => Instruction::Constant(std::f64::consts::E),
            _ if valid_identifier(name) => Instruction::Parameter(name.into()),
            _ => return Err(MaterialError::UnexpectedToken(start)),
        };
        self.push(instruction)
    }

    fn parse_number(&mut self) -> Result<(), MaterialError> {
        let start = self.offset;
        let bytes = self.source.as_bytes();
        while self.offset < bytes.len()
            && (bytes[self.offset].is_ascii_digit() || bytes[self.offset] == b'.')
        {
            self.offset += 1;
        }
        if self.offset < bytes.len() && matches!(bytes[self.offset], b'e' | b'E') {
            self.offset += 1;
            if self.offset < bytes.len() && matches!(bytes[self.offset], b'+' | b'-') {
                self.offset += 1;
            }
            while self.offset < bytes.len() && bytes[self.offset].is_ascii_digit() {
                self.offset += 1;
            }
        }
        let value = self.source[start..self.offset]
            .parse::<f64>()
            .map_err(|_| MaterialError::UnexpectedToken(start))?;
        if !value.is_finite() {
            return Err(MaterialError::InvalidValue);
        }
        self.push(Instruction::Constant(value))
    }

    fn parse_identifier(&mut self) -> Result<&'a str, MaterialError> {
        let start = self.offset;
        let bytes = self.source.as_bytes();
        if self.offset >= bytes.len()
            || !(bytes[self.offset].is_ascii_alphabetic() || bytes[self.offset] == b'_')
        {
            return Err(MaterialError::UnexpectedToken(self.offset));
        }
        self.offset += 1;
        while self.offset < bytes.len()
            && (bytes[self.offset].is_ascii_alphanumeric() || bytes[self.offset] == b'_')
        {
            self.offset += 1;
        }
        Ok(&self.source[start..self.offset])
    }

    fn peek_operator(&self) -> Option<(Instruction, u8, bool)> {
        Some(match self.peek()? {
            '+' => (Instruction::Add, 1, false),
            '-' => (Instruction::Sub, 1, false),
            '*' => (Instruction::Mul, 2, false),
            '/' => (Instruction::Div, 2, false),
            '^' => (Instruction::Pow, 3, true),
            _ => return None,
        })
    }

    fn push(&mut self, instruction: Instruction) -> Result<(), MaterialError> {
        if self.program.len() >= MAX_FORMULA_OPS {
            return Err(MaterialError::TooComplex);
        }
        self.program.push(instruction);
        Ok(())
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.offset += self.peek().unwrap().len_utf8();
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.offset += expected.len_utf8();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.source[self.offset..].chars().next()
    }
}

pub fn valid_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
        && name.len() <= 24
}

pub fn reserved_identifier(name: &str) -> bool {
    matches!(name, "x" | "y" | "r" | "theta" | "pi" | "e") || Function::named(name).is_ok()
}

fn replace_identifier(source: &str, old: &str, new: &str) -> String {
    let mut result = String::with_capacity(source.len() + new.len().saturating_sub(old.len()));
    let mut offset = 0;
    while offset < source.len() {
        let Some(character) = source[offset..].chars().next() else {
            break;
        };
        if character.is_ascii_alphabetic() || character == '_' {
            let start = offset;
            offset += character.len_utf8();
            while let Some(character) = source[offset..].chars().next() {
                if !character.is_ascii_alphanumeric() && character != '_' {
                    break;
                }
                offset += character.len_utf8();
            }
            let identifier = &source[start..offset];
            result.push_str(if identifier == old { new } else { identifier });
        } else {
            result.push(character);
            offset += character.len_utf8();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f64, y: f64) -> MaterialCoordinates {
        MaterialCoordinates {
            x,
            y,
            r: x.hypot(y),
            theta: y.atan2(x),
        }
    }

    #[test]
    fn expressions_use_precedence_coordinates_functions_and_parameters() {
        let formula = MaterialFormula::parse("max(2 + 3 * x^2, sqrt(r)) + R").unwrap();
        let parameters = vec![MaterialParameter {
            name: "R".into(),
            value: 0.5,
        }];
        assert!((formula.evaluate(at(2.0, 0.0), &parameters).unwrap() - 14.5).abs() < 1e-12);
        let power = MaterialFormula::parse("2^3^2").unwrap();
        assert_eq!(power.evaluate(at(0.0, 0.0), &[]).unwrap(), 512.0);
        let unary = MaterialFormula::parse("-2^2 + 2^-2").unwrap();
        assert_eq!(unary.evaluate(at(0.0, 0.0), &[]).unwrap(), -3.75);
    }

    #[test]
    fn rigid_frames_preserve_world_units() {
        let frame = MaterialFrame {
            origin: Point2::new(2.0, 3.0),
            angle_radians: std::f64::consts::FRAC_PI_2,
            attachment: MaterialFrameAttachment::FollowRegion,
        };
        let coordinates = frame.coordinates(Point2::new(2.0, 5.0));
        assert!((coordinates.x - 2.0).abs() < 1e-12);
        assert!(coordinates.y.abs() < 1e-12);
        assert!((coordinates.r - 2.0).abs() < 1e-12);
    }

    #[test]
    fn expressions_reject_bad_domains_missing_parameters_and_limits() {
        let formula = MaterialFormula::parse("sqrt(-1)").unwrap();
        assert_eq!(
            formula.evaluate(at(0.0, 0.0), &[]),
            Err(MaterialError::InvalidValue)
        );
        let missing = MaterialFormula::parse("R + 1").unwrap();
        assert_eq!(
            missing.evaluate(at(0.0, 0.0), &[]),
            Err(MaterialError::MissingParameter("R".into()))
        );
        assert_eq!(
            MaterialFormula::parse("x".repeat(MAX_FORMULA_BYTES + 1)),
            Err(MaterialError::FormulaTooLong)
        );
    }

    #[test]
    fn parameter_rename_rewrites_identifiers_without_touching_longer_names() {
        let field = ScalarField::formula("R + R_outer + sin(R)").unwrap();
        let renamed = field.rename_parameter("R", "radius").unwrap();
        assert_eq!(renamed.source(), Some("radius + R_outer + sin(radius)"));
        let parameters = [
            MaterialParameter {
                name: "radius".into(),
                value: 2.0,
            },
            MaterialParameter {
                name: "R_outer".into(),
                value: 3.0,
            },
        ];
        assert!(
            (renamed.evaluate(at(0.0, 0.0), &parameters).unwrap() - (5.0 + 2.0_f64.sin())).abs()
                < 1e-12
        );
    }
}
