use pillar_shared::proto::NodeStatus;

use super::db::AlertRuleRow;

#[derive(Debug, Clone, Copy)]
pub enum Operator {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
}

impl Operator {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "eq" => Some(Self::Eq),
            "neq" => Some(Self::Neq),
            "gt" => Some(Self::Gt),
            "gte" => Some(Self::Gte),
            "lt" => Some(Self::Lt),
            "lte" => Some(Self::Lte),
            _ => None,
        }
    }
}

pub enum FieldValue {
    Str(String),
    Num(f64),
    Bool(bool),
}

impl std::fmt::Display for FieldValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldValue::Str(s) => f.write_str(s),
            FieldValue::Num(v) => write!(f, "{v}"),
            FieldValue::Bool(b) => write!(f, "{b}"),
        }
    }
}

pub fn extract_field(status: &NodeStatus, field: &str) -> Option<FieldValue> {
    match field {
        "state" => Some(FieldValue::Str(status.state.clone())),
        "crash_looping" => Some(FieldValue::Bool(status.crash_looping)),
        "slots_behind" => Some(FieldValue::Num(status.slots_behind as f64)),
        "cpu_usage_percent" => Some(FieldValue::Num(status.cpu_usage_percent)),
        "memory_percent" if status.memory_total_bytes > 0 => Some(FieldValue::Num(
            status.memory_used_bytes as f64 / status.memory_total_bytes as f64 * 100.0,
        )),
        "memory_percent" => Some(FieldValue::Num(0.0)),
        "disk_percent" if status.disk_total_bytes > 0 => Some(FieldValue::Num(
            status.disk_used_bytes as f64 / status.disk_total_bytes as f64 * 100.0,
        )),
        "disk_percent" => Some(FieldValue::Num(0.0)),
        "version_mismatch" => Some(FieldValue::Bool(status.version_mismatch)),
        "agent_uptime_secs" => Some(FieldValue::Num(status.agent_uptime_secs as f64)),
        "healthy" => Some(FieldValue::Bool(status.healthy)),
        "restart_count" => Some(FieldValue::Num(status.restart_count as f64)),
        _ => None,
    }
}

pub fn evaluate_condition(value: &FieldValue, op: Operator, threshold: &str) -> bool {
    match value {
        FieldValue::Str(s) => matches!(op, Operator::Eq if s == threshold)
            || matches!(op, Operator::Neq if s != threshold),
        FieldValue::Bool(b) => {
            let t = threshold == "true";
            matches!(op, Operator::Eq if *b == t) || matches!(op, Operator::Neq if *b != t)
        }
        FieldValue::Num(v) => {
            let Ok(t) = threshold.parse::<f64>() else {
                return false;
            };
            match op {
                Operator::Eq => (*v - t).abs() < f64::EPSILON,
                Operator::Neq => (*v - t).abs() >= f64::EPSILON,
                Operator::Gt => *v > t,
                Operator::Gte => *v >= t,
                Operator::Lt => *v < t,
                Operator::Lte => *v <= t,
            }
        }
    }
}

pub fn rule_applies_to_node(rule: &AlertRuleRow, node_id: &str) -> bool {
    match &rule.node_id_filter {
        None => true,
        Some(f) if f.is_empty() => true,
        Some(f) => f.split(',').any(|id| id.trim() == node_id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_eq() {
        let val = FieldValue::Str("off".into());
        assert!(evaluate_condition(&val, Operator::Eq, "off"));
        assert!(!evaluate_condition(&val, Operator::Eq, "healthy"));
        assert!(evaluate_condition(&val, Operator::Neq, "healthy"));
    }

    #[test]
    fn bool_eq() {
        assert!(evaluate_condition(&FieldValue::Bool(true), Operator::Eq, "true"));
        assert!(!evaluate_condition(&FieldValue::Bool(true), Operator::Eq, "false"));
    }

    #[test]
    fn numeric_comparisons() {
        let val = FieldValue::Num(95.0);
        assert!(evaluate_condition(&val, Operator::Gt, "90"));
        assert!(!evaluate_condition(&val, Operator::Gt, "95"));
        assert!(evaluate_condition(&val, Operator::Gte, "95"));
        assert!(evaluate_condition(&val, Operator::Lt, "100"));
    }

    #[test]
    fn field_extraction() {
        let status = NodeStatus {
            state: "healthy".into(),
            cpu_usage_percent: 42.5,
            memory_used_bytes: 500,
            memory_total_bytes: 1000,
            ..Default::default()
        };
        assert!(matches!(extract_field(&status, "state"), Some(FieldValue::Str(s)) if s == "healthy"));
        assert!(matches!(extract_field(&status, "cpu_usage_percent"), Some(FieldValue::Num(v)) if (v - 42.5).abs() < f64::EPSILON));
        assert!(matches!(extract_field(&status, "memory_percent"), Some(FieldValue::Num(v)) if (v - 50.0).abs() < f64::EPSILON));
        assert!(extract_field(&status, "nonexistent").is_none());
    }

    #[test]
    fn node_filter() {
        let mut rule = AlertRuleRow {
            id: "t".into(), name: "t".into(), description: String::new(),
            field: "state".into(), operator: "eq".into(), threshold: "off".into(),
            node_id_filter: None, enabled: true, severity: "warning".into(),
            cooldown_secs: 0, is_default: false, created_at: 0, updated_at: 0,
        };
        assert!(rule_applies_to_node(&rule, "any"));
        rule.node_id_filter = Some("node-1, node-2".into());
        assert!(rule_applies_to_node(&rule, "node-1"));
        assert!(!rule_applies_to_node(&rule, "node-3"));
    }
}
