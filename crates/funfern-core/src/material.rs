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
    UnsupportedMaterialLaw,
    /// A law a physics skin change cannot carry, named by what blocks it so the
    /// editor can say which slot to clear.
    UnconvertibleMaterialLaw(&'static str),
    /// A material already carries as many named parameters as it can hold.
    ParameterLimit,
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
            Self::UnsupportedMaterialLaw => {
                write!(
                    f,
                    "material law is authored but not executable by the legacy solver"
                )
            }
            Self::UnconvertibleMaterialLaw(blocked) => {
                write!(f, "{blocked} cannot cross a physics skin yet")
            }
            Self::ParameterLimit => {
                write!(
                    f,
                    "a material holds at most {MAX_MATERIAL_PARAMETERS} parameters"
                )
            }
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

    /// Rotates a tensor expressed in this frame's local axes into world axes.
    pub fn tensor_from_local(self, x: f64, y: f64) -> SymmetricTensor2 {
        SymmetricTensor2::from_principal(x, y, self.angle_radians)
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

    /// Whether the field is the same at every point: a constant, or a formula
    /// that reads only parameters and constants.
    pub fn spatially_constant(&self) -> bool {
        match self {
            Self::Constant(_) => true,
            Self::Formula(formula) => formula.spatially_constant(),
        }
    }

    /// Evaluates a field that does not vary in space, so the coordinates it is
    /// given do not matter. A field that does vary is refused rather than
    /// sampled somewhere arbitrary.
    pub fn evaluate_constant(
        &self,
        parameters: &[MaterialParameter],
    ) -> Result<f64, MaterialError> {
        if !self.spatially_constant() {
            return Err(MaterialError::InvalidValue);
        }
        self.evaluate(
            MaterialCoordinates {
                x: 0.0,
                y: 0.0,
                r: 0.0,
                theta: 0.0,
            },
            parameters,
        )
    }

    pub fn rename_parameter(&self, old: &str, new: &str) -> Result<Self, MaterialError> {
        match self {
            Self::Constant(value) => Ok(Self::Constant(*value)),
            Self::Formula(formula) => Self::formula(replace_identifier(formula.source(), old, new)),
        }
    }

    /// Builds a reciprocal expression and performs the small, structural
    /// simplifications used when converting between physics material laws.
    pub(crate) fn reciprocal(&self) -> Result<Self, MaterialError> {
        Self::from_transformed_expression(Expression::Binary {
            operator: BinaryOperator::Div,
            left: Box::new(Expression::Constant(1.0)),
            right: Box::new(self.expression()?),
        })
    }

    /// Builds a product and performs the small, structural simplifications used
    /// when converting between physics material laws.
    pub(crate) fn multiply(&self, right: &Self) -> Result<Self, MaterialError> {
        Self::from_transformed_expression(Expression::Binary {
            operator: BinaryOperator::Mul,
            left: Box::new(self.expression()?),
            right: Box::new(right.expression()?),
        })
    }

    /// Builds a quotient and performs the small, structural simplifications used
    /// when converting between physics material laws.
    pub(crate) fn divide(&self, right: &Self) -> Result<Self, MaterialError> {
        Self::from_transformed_expression(Expression::Binary {
            operator: BinaryOperator::Div,
            left: Box::new(self.expression()?),
            right: Box::new(right.expression()?),
        })
    }

    fn expression(&self) -> Result<Expression, MaterialError> {
        match self {
            Self::Constant(value) => Ok(Expression::Constant(*value)),
            Self::Formula(formula) => parse_expression(formula.source()),
        }
    }

    fn from_transformed_expression(expression: Expression) -> Result<Self, MaterialError> {
        let expression = expression.simplify();
        if let Expression::Constant(value) = expression {
            return value
                .is_finite()
                .then_some(Self::Constant(value))
                .ok_or(MaterialError::InvalidValue);
        }
        Ok(Self::Formula(MaterialFormula::from_expression(expression)?))
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
    /// Ratio of the wave speed along the material-frame x axis to that along y.
    pub axis_ratio: f64,
}

impl EvaluatedMaterial {
    pub fn valid(self) -> bool {
        self.mass_density.is_finite()
            && self.mass_density > 0.0
            && self.stiffness.is_finite()
            && self.stiffness > 0.0
            && self.damping.is_finite()
            && self.damping >= 0.0
            && self.axis_ratio.is_finite()
            && self.axis_ratio >= 1.0
    }
}

/// A symmetric 2x2 tensor stored without redundant entries.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SymmetricTensor2 {
    pub xx: f64,
    pub xy: f64,
    pub yy: f64,
}

impl SymmetricTensor2 {
    pub const fn new(xx: f64, xy: f64, yy: f64) -> Self {
        Self { xx, xy, yy }
    }

    pub const fn isotropic(value: f64) -> Self {
        Self::new(value, 0.0, value)
    }

    pub fn from_principal(x: f64, y: f64, angle_radians: f64) -> Self {
        let (sin, cos) = angle_radians.sin_cos();
        Self {
            xx: x * cos * cos + y * sin * sin,
            xy: (x - y) * sin * cos,
            yy: x * sin * sin + y * cos * cos,
        }
    }

    pub fn apply(self, vector: Point2) -> Point2 {
        Point2::new(
            self.xx * vector.x + self.xy * vector.y,
            self.xy * vector.x + self.yy * vector.y,
        )
    }

    pub fn quadratic_form(self, vector: Point2) -> f64 {
        vector.dot(self.apply(vector))
    }

    pub fn contract(self, other: Self) -> f64 {
        self.xx * other.xx + 2.0 * self.xy * other.xy + self.yy * other.yy
    }

    pub fn determinant(self) -> f64 {
        self.xx * self.yy - self.xy * self.xy
    }

    /// The inverse of a finite positive-definite tensor. A constitutive
    /// coefficient and its inverse are both exact data in this design, so the
    /// same arithmetic is shared rather than rewritten at each use.
    pub fn inverse(self) -> Option<Self> {
        let determinant = self.determinant();
        self.finite_spd().then(|| {
            Self::new(
                self.yy / determinant,
                -self.xy / determinant,
                self.xx / determinant,
            )
        })
    }

    pub fn eigenvalues(self) -> [f64; 2] {
        let mean = 0.5 * (self.xx + self.yy);
        let radius = (0.25 * (self.xx - self.yy).powi(2) + self.xy * self.xy).sqrt();
        [mean - radius, mean + radius]
    }

    pub fn finite_spd(self) -> bool {
        self.xx.is_finite()
            && self.xy.is_finite()
            && self.yy.is_finite()
            && self.xx > 0.0
            && self.determinant().is_finite()
            && self.determinant() > 0.0
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
        let expression = parse_expression(&source)?;
        Self::from_source_and_expression(source, &expression)
    }

    fn from_expression(expression: Expression) -> Result<Self, MaterialError> {
        let source = expression.to_string();
        Self::parse(source)
    }

    fn from_source_and_expression(
        source: String,
        expression: &Expression,
    ) -> Result<Self, MaterialError> {
        if source.trim().is_empty() {
            return Err(MaterialError::EmptyFormula);
        }
        if source.len() > MAX_FORMULA_BYTES {
            return Err(MaterialError::FormulaTooLong);
        }
        let mut program = Vec::new();
        expression.compile(&mut program)?;
        let mut parameters = Vec::new();
        for instruction in &program {
            if let Instruction::Parameter(name) = instruction
                && !parameters.contains(name)
            {
                parameters.push(name.clone());
            }
        }
        Ok(Self {
            source,
            program,
            parameters,
        })
    }

    /// Whether the program never reads a coordinate.
    pub fn spatially_constant(&self) -> bool {
        !self.program.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::X | Instruction::Y | Instruction::R | Instruction::Theta
            )
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

#[derive(Clone, Debug, PartialEq)]
enum Expression {
    Constant(f64),
    X,
    Y,
    R,
    Theta,
    Parameter(String),
    Neg(Box<Self>),
    Binary {
        operator: BinaryOperator,
        left: Box<Self>,
        right: Box<Self>,
    },
    Function {
        function: Function,
        arguments: Vec<Self>,
    },
}

impl Expression {
    fn compile(&self, program: &mut Vec<Instruction>) -> Result<(), MaterialError> {
        match self {
            Self::Constant(value) => push_instruction(program, Instruction::Constant(*value)),
            Self::X => push_instruction(program, Instruction::X),
            Self::Y => push_instruction(program, Instruction::Y),
            Self::R => push_instruction(program, Instruction::R),
            Self::Theta => push_instruction(program, Instruction::Theta),
            Self::Parameter(name) => {
                push_instruction(program, Instruction::Parameter(name.clone()))
            }
            Self::Neg(value) => {
                value.compile(program)?;
                push_instruction(program, Instruction::Neg)
            }
            Self::Binary {
                operator,
                left,
                right,
            } => {
                left.compile(program)?;
                right.compile(program)?;
                push_instruction(program, operator.instruction())
            }
            Self::Function {
                function,
                arguments,
            } => {
                for argument in arguments {
                    argument.compile(program)?;
                }
                push_instruction(program, Instruction::Function(*function))
            }
        }
    }

    fn simplify(self) -> Self {
        let expression = match self {
            Self::Neg(value) => Self::Neg(Box::new(value.simplify())),
            Self::Binary {
                operator,
                left,
                right,
            } => Self::Binary {
                operator,
                left: Box::new(left.simplify()),
                right: Box::new(right.simplify()),
            },
            Self::Function {
                function,
                arguments,
            } => Self::Function {
                function,
                arguments: arguments.into_iter().map(Self::simplify).collect(),
            },
            expression => expression,
        };
        let mut expression = expression;
        while let Some(simplified) = expression.simplify_once() {
            if simplified == expression {
                break;
            }
            expression = simplified;
        }
        expression
    }

    fn simplify_once(&self) -> Option<Self> {
        match self {
            Self::Neg(value) => match value.as_ref() {
                Self::Constant(value) if (-value).is_finite() => Some(Self::Constant(-value)),
                Self::Neg(inner) => Some((**inner).clone()),
                _ => None,
            },
            Self::Binary {
                operator,
                left,
                right,
            } => {
                if let (Self::Constant(left), Self::Constant(right)) =
                    (left.as_ref(), right.as_ref())
                {
                    let value = operator.evaluate(*left, *right);
                    if value.is_finite() {
                        return Some(Self::Constant(value));
                    }
                }
                match operator {
                    BinaryOperator::Add if right.is_zero() => Some((**left).clone()),
                    BinaryOperator::Add if left.is_zero() => Some((**right).clone()),
                    BinaryOperator::Sub if right.is_zero() => Some((**left).clone()),
                    BinaryOperator::Mul if left.is_zero() || right.is_zero() => {
                        Some(Self::Constant(0.0))
                    }
                    BinaryOperator::Mul if right.is_one() => Some((**left).clone()),
                    BinaryOperator::Mul if left.is_one() => Some((**right).clone()),
                    BinaryOperator::Div if left.is_zero() && !right.is_zero() => {
                        Some(Self::Constant(0.0))
                    }
                    BinaryOperator::Div if right.is_one() => Some((**left).clone()),
                    BinaryOperator::Div if left.is_one() => {
                        if let Self::Binary {
                            operator: BinaryOperator::Div,
                            left: inner_left,
                            right: inner_right,
                        } = right.as_ref()
                            && inner_left.is_one()
                        {
                            return Some((**inner_right).clone());
                        }
                        None
                    }
                    BinaryOperator::Mul => cancel_product(left, right),
                    BinaryOperator::Div => cancel_quotient(left, right),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn is_zero(&self) -> bool {
        matches!(self, Self::Constant(value) if *value == 0.0)
    }

    fn is_one(&self) -> bool {
        matches!(self, Self::Constant(value) if *value == 1.0)
    }
}

fn cancel_product(left: &Expression, right: &Expression) -> Option<Expression> {
    for (quotient, factor) in [(left, right), (right, left)] {
        if let Expression::Binary {
            operator: BinaryOperator::Div,
            left: numerator,
            right: denominator,
        } = quotient
            && denominator.as_ref() == factor
        {
            return Some((**numerator).clone());
        }
    }
    None
}

fn cancel_quotient(left: &Expression, right: &Expression) -> Option<Expression> {
    if let Expression::Binary {
        operator: BinaryOperator::Mul,
        left: first,
        right: second,
    } = left
    {
        if first.as_ref() == right {
            return Some((**second).clone());
        }
        if second.as_ref() == right {
            return Some((**first).clone());
        }
    }
    None
}

impl std::fmt::Display for Expression {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fmt_expression(formatter)
    }
}

impl Expression {
    fn fmt_expression(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Constant(value) => write!(formatter, "{value}"),
            Self::X => formatter.write_str("x"),
            Self::Y => formatter.write_str("y"),
            Self::R => formatter.write_str("r"),
            Self::Theta => formatter.write_str("theta"),
            Self::Parameter(name) => formatter.write_str(name),
            Self::Neg(value) => write!(formatter, "-({value})"),
            Self::Binary {
                operator,
                left,
                right,
            } => {
                let precedence = operator.precedence();
                let left_precedence = left.precedence();
                let right_precedence = right.precedence();
                let left_parenthesized = left_precedence < precedence
                    || *operator == BinaryOperator::Pow && left_precedence == precedence;
                let right_parenthesized = right_precedence < precedence
                    || right_precedence == precedence && !matches!(operator, BinaryOperator::Pow);
                left.fmt_operand(formatter, left_parenthesized)?;
                write!(formatter, " {} ", operator.symbol())?;
                right.fmt_operand(formatter, right_parenthesized)
            }
            Self::Function {
                function,
                arguments,
            } => {
                write!(formatter, "{}(", function.name())?;
                for (index, argument) in arguments.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{argument}")?;
                }
                formatter.write_str(")")
            }
        }
    }

    fn precedence(&self) -> u8 {
        match self {
            Self::Binary { operator, .. } => operator.precedence(),
            Self::Neg(_) => 3,
            Self::Constant(value) if value.is_sign_negative() => 3,
            _ => 4,
        }
    }

    fn fmt_operand(
        &self,
        formatter: &mut std::fmt::Formatter<'_>,
        parenthesized: bool,
    ) -> std::fmt::Result {
        if parenthesized {
            formatter.write_str("(")?;
        }
        self.fmt_expression(formatter)?;
        if parenthesized {
            formatter.write_str(")")?;
        }
        Ok(())
    }
}

fn push_instruction(
    program: &mut Vec<Instruction>,
    instruction: Instruction,
) -> Result<(), MaterialError> {
    if program.len() >= MAX_FORMULA_OPS {
        return Err(MaterialError::TooComplex);
    }
    program.push(instruction);
    Ok(())
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
enum BinaryOperator {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

impl BinaryOperator {
    const fn instruction(self) -> Instruction {
        match self {
            Self::Add => Instruction::Add,
            Self::Sub => Instruction::Sub,
            Self::Mul => Instruction::Mul,
            Self::Div => Instruction::Div,
            Self::Pow => Instruction::Pow,
        }
    }

    const fn symbol(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Pow => "^",
        }
    }

    const fn precedence(self) -> u8 {
        match self {
            Self::Add | Self::Sub => 1,
            Self::Mul | Self::Div => 2,
            Self::Pow => 3,
        }
    }

    fn evaluate(self, left: f64, right: f64) -> f64 {
        match self {
            Self::Add => left + right,
            Self::Sub => left - right,
            Self::Mul => left * right,
            Self::Div => left / right,
            Self::Pow => left.powf(right),
        }
    }
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

    const fn name(self) -> &'static str {
        match self {
            Self::Sqrt => "sqrt",
            Self::Abs => "abs",
            Self::Sin => "sin",
            Self::Cos => "cos",
            Self::Tan => "tan",
            Self::Exp => "exp",
            Self::Log => "log",
            Self::Min => "min",
            Self::Max => "max",
            Self::Clamp => "clamp",
            Self::Smoothstep => "smoothstep",
        }
    }
}

struct Parser<'a> {
    source: &'a str,
    offset: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            offset: 0,
            depth: 0,
        }
    }

    fn parse_expression(&mut self, minimum_precedence: u8) -> Result<Expression, MaterialError> {
        self.depth += 1;
        if self.depth > MAX_PARSE_DEPTH {
            return Err(MaterialError::TooComplex);
        }
        self.skip_space();
        let mut expression = if self.consume('-') {
            Expression::Neg(Box::new(self.parse_expression(3)?))
        } else if self.consume('+') {
            self.parse_expression(3)?
        } else {
            self.parse_primary()?
        };
        loop {
            self.skip_space();
            let Some((operator, precedence, right_associative)) = self.peek_operator() else {
                break;
            };
            if precedence < minimum_precedence {
                break;
            }
            self.offset += 1;
            let right = self.parse_expression(if right_associative {
                precedence
            } else {
                precedence + 1
            })?;
            expression = Expression::Binary {
                operator,
                left: Box::new(expression),
                right: Box::new(right),
            };
        }
        self.depth -= 1;
        Ok(expression)
    }

    fn parse_primary(&mut self) -> Result<Expression, MaterialError> {
        self.skip_space();
        if self.consume('(') {
            let expression = self.parse_expression(0)?;
            self.skip_space();
            if !self.consume(')') {
                return Err(MaterialError::UnexpectedToken(self.offset));
            }
            return Ok(expression);
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
            let mut arguments = Vec::with_capacity(expected);
            self.skip_space();
            if !self.consume(')') {
                loop {
                    arguments.push(self.parse_expression(0)?);
                    self.skip_space();
                    if self.consume(')') {
                        break;
                    }
                    if !self.consume(',') {
                        return Err(MaterialError::UnexpectedToken(self.offset));
                    }
                }
            }
            if arguments.len() != expected {
                return Err(MaterialError::InvalidArgumentCount(name.into()));
            }
            return Ok(Expression::Function {
                function,
                arguments,
            });
        }
        let expression = match name {
            "x" => Expression::X,
            "y" => Expression::Y,
            "r" => Expression::R,
            "theta" => Expression::Theta,
            "pi" => Expression::Constant(std::f64::consts::PI),
            "e" => Expression::Constant(std::f64::consts::E),
            _ if valid_identifier(name) => Expression::Parameter(name.into()),
            _ => return Err(MaterialError::UnexpectedToken(start)),
        };
        Ok(expression)
    }

    fn parse_number(&mut self) -> Result<Expression, MaterialError> {
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
        Ok(Expression::Constant(value))
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

    fn peek_operator(&self) -> Option<(BinaryOperator, u8, bool)> {
        Some(match self.peek()? {
            '+' => (BinaryOperator::Add, 1, false),
            '-' => (BinaryOperator::Sub, 1, false),
            '*' => (BinaryOperator::Mul, 2, false),
            '/' => (BinaryOperator::Div, 2, false),
            '^' => (BinaryOperator::Pow, 3, true),
            _ => return None,
        })
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

fn parse_expression(source: &str) -> Result<Expression, MaterialError> {
    if source.trim().is_empty() {
        return Err(MaterialError::EmptyFormula);
    }
    if source.len() > MAX_FORMULA_BYTES {
        return Err(MaterialError::FormulaTooLong);
    }
    let mut parser = Parser::new(source);
    let expression = parser.parse_expression(0)?;
    parser.skip_space();
    if parser.offset != source.len() {
        return Err(MaterialError::UnexpectedToken(parser.offset));
    }
    Ok(expression)
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

    #[test]
    fn transformed_formulas_simplify_without_rewriting_ordinary_input() {
        let field = ScalarField::formula(" 2 + 3*x ").unwrap();
        assert_eq!(field.source(), Some(" 2 + 3*x "));

        let reciprocal = field.reciprocal().unwrap();
        assert_eq!(reciprocal.source(), Some("1 / (2 + 3 * x)"));
        let restored = reciprocal.reciprocal().unwrap();
        assert_eq!(restored.source(), Some("2 + 3 * x"));
        for x in [-0.4, 0.0, 0.75] {
            assert_eq!(
                restored.evaluate(at(x, 0.0), &[]),
                field.evaluate(at(x, 0.0), &[])
            );
        }

        let damping = ScalarField::formula("0.2 + y*y").unwrap();
        let density = ScalarField::formula("2 + x*x").unwrap();
        let zero_loss = ScalarField::constant(0.0).divide(&density).unwrap();
        assert_eq!(zero_loss, ScalarField::constant(0.0));
        assert_eq!(
            zero_loss.multiply(&density).unwrap(),
            ScalarField::constant(0.0)
        );
        let normalized = damping.divide(&density).unwrap();
        let round_trip = normalized.multiply(&density).unwrap();
        assert_eq!(round_trip.source(), Some("0.2 + y * y"));
        for (x, y) in [(-0.5, 0.2), (0.0, 0.0), (0.8, -0.4)] {
            assert_eq!(
                round_trip.evaluate(at(x, y), &[]),
                damping.evaluate(at(x, y), &[])
            );
        }

        let precedence = ScalarField::formula("(-2)^x + 2^-2 + x/(2/y)").unwrap();
        let precedence_round_trip = precedence.reciprocal().unwrap().reciprocal().unwrap();
        assert_eq!(
            precedence_round_trip.source(),
            Some("(-2) ^ x + 0.25 + x / (2 / y)")
        );
        assert_eq!(
            precedence_round_trip.evaluate(at(2.0, 4.0), &[]),
            precedence.evaluate(at(2.0, 4.0), &[])
        );
    }

    #[test]
    fn symmetric_tensor_rotation_preserves_principal_values_and_quadratic_form() {
        let angle = 0.37;
        let tensor = SymmetricTensor2::from_principal(6.0, 1.5, angle);
        let eigenvalues = tensor.eigenvalues();
        assert!((eigenvalues[0] - 1.5).abs() < 1.0e-12);
        assert!((eigenvalues[1] - 6.0).abs() < 1.0e-12);
        assert!((tensor.determinant() - 9.0).abs() < 1.0e-12);
        assert!(tensor.finite_spd());

        let axis = Point2::new(angle.cos(), angle.sin());
        assert!((tensor.quadratic_form(axis) - 6.0).abs() < 1.0e-12);
        assert!(!SymmetricTensor2::new(1.0, 2.0, 1.0).finite_spd());
    }
}
