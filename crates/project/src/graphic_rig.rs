//! Bounded, declarative relationships between graphic properties. No executable scripts.
use crate::gtemplate::layer_uid;
use crate::{EffectInstance, Param, ParamValue};
use filmcraft_geom::Vec2;
use filmcraft_time::Tick;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Component {
    #[default]
    Value,
    X,
    Y,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Op {
    Constant {
        value: f64,
    },
    Parameter {
        layer: u64,
        param: String,
        #[serde(default)]
        component: Component,
    },
    Bounds {
        layer: u64,
        edge: usize,
    },
    Add,
    Subtract,
    Multiply,
    Divide,
    Min,
    Max,
    Round,
    Equal,
    Select,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub layer: u64,
    pub param: String,
    #[serde(default)]
    pub component: Component,
    pub expression: Vec<Op>,
}

fn one() -> f64 {
    1.0
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphicRig {
    pub bindings: Vec<Binding>,
    /// Maps computed design positions and scales to the instantiated canvas.
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub offset: Vec2,
}

impl GraphicRig {
    pub fn validate(&self, layers: &[EffectInstance]) -> Result<(), String> {
        if self.bindings.len() > 512 || !self.scale.is_finite() || self.scale <= 0.0 || !self.offset.x.is_finite() || !self.offset.y.is_finite() {
            return Err("invalid graphic rig size or canvas transform".into());
        }
        for b in &self.bindings {
            let p = layers.iter().find(|e| layer_uid(e) == b.layer).and_then(|e| e.params.get(&b.param)).ok_or("rig target is missing")?;
            if !matches!((&p.value, b.component), (ParamValue::Float(_), Component::Value) | (ParamValue::Vec2(_), Component::X | Component::Y)) {
                return Err("rig target must be a scalar or point component".into());
            }
            if b.expression.is_empty() || b.expression.len() > 512 {
                return Err("invalid rig expression length".into());
            }
            let mut depth = 0usize;
            for op in &b.expression {
                let (consume, produce) = match op {
                    Op::Constant { value } => {
                        if !value.is_finite() {
                            return Err("nonfinite rig constant".into());
                        }
                        (0, 1)
                    }
                    Op::Parameter { layer, param, component } => {
                        let p = layers.iter().find(|e| layer_uid(e) == *layer).and_then(|e| e.params.get(param)).ok_or("rig input is missing")?;
                        if number(&p.value, *component).is_none() {
                            return Err("rig input is not numeric".into());
                        }
                        (0, 1)
                    }
                    Op::Bounds { layer, edge } => {
                        if *edge > 3 || !layers.iter().any(|e| layer_uid(e) == *layer) {
                            return Err("invalid rig bounds reference".into());
                        }
                        (0, 1)
                    }
                    Op::Round => (1, 1),
                    Op::Select => (3, 1),
                    _ => (2, 1),
                };
                depth = depth.checked_sub(consume).ok_or("rig expression stack underflow")? + produce;
            }
            if depth != 1 {
                return Err("rig expression must yield one number".into());
            }
        }
        Ok(())
    }

    /// Bindings run in document order against a private copy. A later binding may read a prior
    /// result; there is no recursion or implicit dependency traversal.
    pub fn evaluate(&self, layers: &[EffectInstance], time: Tick, bounds: impl Fn(&EffectInstance) -> [f64; 4]) -> Result<Vec<EffectInstance>, String> {
        self.validate(layers)?;
        let mut result = layers.to_vec();
        for b in &self.bindings {
            let mut stack = Vec::new();
            for op in &b.expression {
                let value = match op {
                    Op::Constant { value } => *value,
                    Op::Parameter { layer, param, component } => {
                        let p = result.iter().find(|e| layer_uid(e) == *layer).and_then(|e| e.params.get(param)).ok_or("rig input is missing")?;
                        number(&p.value_at(time), *component).ok_or("invalid rig input")?
                    }
                    Op::Bounds { layer, edge } => {
                        let e = result.iter().find(|e| layer_uid(e) == *layer).ok_or("rig bounds layer is missing")?;
                        *bounds(e).get(*edge).ok_or("invalid rig bounds edge")?
                    }
                    Op::Round => pop(&mut stack)?.round(),
                    Op::Select => {
                        let no = pop(&mut stack)?;
                        let yes = pop(&mut stack)?;
                        let condition = pop(&mut stack)?;
                        if condition != 0.0 { yes } else { no }
                    }
                    op => {
                        let rhs = pop(&mut stack)?;
                        let lhs = pop(&mut stack)?;
                        match op {
                            Op::Add => lhs + rhs,
                            Op::Subtract => lhs - rhs,
                            Op::Multiply => lhs * rhs,
                            Op::Divide if rhs != 0.0 => lhs / rhs,
                            Op::Min => lhs.min(rhs),
                            Op::Max => lhs.max(rhs),
                            Op::Equal => {
                                if lhs == rhs {
                                    1.0
                                } else {
                                    0.0
                                }
                            }
                            _ => return Err("invalid rig operation".into()),
                        }
                    }
                };
                if !value.is_finite() || value.abs() > 1e9 {
                    return Err("rig result is outside numeric limits".into());
                }
                stack.push(value);
            }
            let mut v = pop(&mut stack)?;
            if b.param == "position" {
                v = v * self.scale
                    + match b.component {
                        Component::X => self.offset.x,
                        Component::Y => self.offset.y,
                        _ => 0.0,
                    };
            }
            if b.param == "scale" || b.param == "scale_width" {
                v *= self.scale;
            }
            if !v.is_finite() || v.abs() > 1e9 {
                return Err("transformed rig result is outside numeric limits".into());
            }
            let p = result.iter_mut().find(|e| layer_uid(e) == b.layer).and_then(|e| e.params.get_mut(&b.param)).ok_or("rig target is missing")?;
            let value = match (p.value_at(time), b.component) {
                (ParamValue::Float(_), Component::Value) => ParamValue::Float(v),
                (ParamValue::Vec2(mut point), Component::X) => {
                    point.x = v;
                    ParamValue::Vec2(point)
                }
                (ParamValue::Vec2(mut point), Component::Y) => {
                    point.y = v;
                    ParamValue::Vec2(point)
                }
                _ => return Err("invalid rig target".into()),
            };
            *p = Param::new(value);
        }
        Ok(result)
    }
}

fn pop(stack: &mut Vec<f64>) -> Result<f64, String> {
    stack.pop().ok_or_else(|| "rig expression stack underflow".into())
}
fn number(value: &ParamValue, component: Component) -> Option<f64> {
    match (value, component) {
        (ParamValue::Vec2(v), Component::X) => Some(v.x),
        (ParamValue::Vec2(v), Component::Y) => Some(v.y),
        (v, Component::Value) => v.as_f64(),
        _ => None,
    }
    .filter(|n| n.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphic::{new_shape_layer, new_text_layer};
    use crate::graphic_design::LayerExtra;

    fn fixture() -> (Vec<EffectInstance>, GraphicRig) {
        let mut text = new_text_layer("Title", Vec2::new(0.0, 0.0), 40.0);
        text.layer = Some(Box::new(LayerExtra { uid: 1, ..Default::default() }));
        let mut shape = new_shape_layer(0, Vec2::new(0.0, 0.0), Vec2::new(10.0, 20.0), vec![]);
        shape.layer = Some(Box::new(LayerExtra { uid: 2, ..Default::default() }));
        let rig = GraphicRig {
            scale: 1.0,
            offset: Vec2::default(),
            bindings: vec![Binding {
                layer: 2,
                param: "size".into(),
                component: Component::X,
                expression: vec![Op::Bounds { layer: 1, edge: 2 }, Op::Constant { value: 20.0 }, Op::Add],
            }],
        };
        (vec![text, shape], rig)
    }

    #[test]
    fn responsive_bounds_are_evaluated_without_mutating_source() {
        let (layers, rig) = fixture();
        let out = rig.evaluate(&layers, Tick::ZERO, |_| [0.0, 0.0, 123.0, 40.0]).unwrap();
        assert_eq!(out[1].params["size"].value, ParamValue::Vec2(Vec2::new(143.0, 20.0)));
        assert_eq!(layers[1].params["size"].value, ParamValue::Vec2(Vec2::new(10.0, 20.0)));
        let back: GraphicRig = serde_json::from_str(&serde_json::to_string(&rig).unwrap()).unwrap();
        assert_eq!(back, rig);
    }

    #[test]
    fn invalid_stack_reference_and_numeric_results_return_errors() {
        let (layers, mut rig) = fixture();
        for expr in [
            vec![Op::Add],
            vec![Op::Bounds { layer: 99, edge: 2 }],
            vec![Op::Bounds { layer: 1, edge: 4 }],
            vec![Op::Constant { value: f64::NAN }],
            vec![Op::Constant { value: 1.0 }, Op::Constant { value: 0.0 }, Op::Divide],
        ] {
            rig.bindings[0].expression = expr;
            assert!(rig.evaluate(&layers, Tick::ZERO, |_| [0.0; 4]).is_err());
        }
        rig.bindings[0].param = "position".into();
        rig.bindings[0].expression = vec![Op::Constant { value: 2.0 }];
        rig.scale = f64::MAX;
        assert!(rig.evaluate(&layers, Tick::ZERO, |_| [0.0; 4]).is_err());
    }

    #[test]
    fn canvas_fit_transforms_computed_positions() {
        let (layers, mut rig) = fixture();
        rig.bindings[0].param = "position".into();
        rig.scale = 0.5;
        rig.offset = Vec2::new(7.0, 9.0);
        let out = rig.evaluate(&layers, Tick::ZERO, |_| [0.0, 0.0, 100.0, 40.0]).unwrap();
        assert_eq!(out[1].params["position"].value, ParamValue::Vec2(Vec2::new(67.0, 0.0)));
    }
}
