//! Entity attribute values with modifiers. Port of typecraft's
//! `physics/attribute.ts`.

#[derive(Debug, Clone, Copy)]
pub struct AttributeModifier {
    pub uuid: &'static str,
    pub amount: f64,
    /// 0 = add to base, 1 = multiply base, 2 = multiply total.
    pub operation: i32,
}

#[derive(Debug, Clone, Default)]
pub struct AttributeValue {
    pub value: f64,
    pub modifiers: Vec<AttributeModifier>,
}

impl AttributeValue {
    pub fn new(base: f64) -> AttributeValue {
        AttributeValue {
            value: base,
            modifiers: Vec::new(),
        }
    }

    /// Final value: op0 (add to base), op1 (multiply base), op2 (multiply total).
    pub fn compute(&self) -> f64 {
        let mut base = self.value;
        for m in &self.modifiers {
            if m.operation == 0 {
                base += m.amount;
            }
        }
        let mut total = base;
        for m in &self.modifiers {
            if m.operation == 1 {
                total += base * m.amount;
            }
        }
        for m in &self.modifiers {
            if m.operation == 2 {
                total += total * m.amount;
            }
        }
        total
    }

    pub fn with_modifier(mut self, modifier: AttributeModifier) -> AttributeValue {
        self.modifiers.push(modifier);
        self
    }

    pub fn without_modifier(mut self, uuid: &str) -> AttributeValue {
        self.modifiers.retain(|m| m.uuid != uuid);
        self
    }

    pub fn has_modifier(&self, uuid: &str) -> bool {
        self.modifiers.iter().any(|m| m.uuid == uuid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_operations_in_order() {
        let attr = AttributeValue::new(0.1).with_modifier(AttributeModifier {
            uuid: "a",
            amount: 0.3,
            operation: 2,
        });
        // op2: total += total * 0.3 = 0.1 * 1.3 = 0.13
        assert!((attr.compute() - 0.13).abs() < 1e-9);
        assert!(attr.has_modifier("a"));
        assert!(!attr.without_modifier("a").has_modifier("a"));
    }
}
