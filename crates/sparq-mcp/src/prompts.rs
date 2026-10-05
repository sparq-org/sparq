//! [SONNET-4.6] sq-sjey1 (gh #3220) The MCP **prompts** surface of the base
//! [`McpServer`](crate::McpServer): a small catalog of canned, dataset-agnostic query
//! prompts a client can offer its user as a starting point.
//!
//! MCP prompts are *user-controlled* templates: `prompts/list` advertises them,
//! `prompts/get` renders one (with arguments) into the messages the client injects. This
//! catalog is deliberately **static text over the tools that already exist** — a prompt
//! tells the model which of this server's tools to call in which order and hands it
//! ready-to-run SPARQL. Nothing here queries the graph, so a `prompts/get` is free and
//! cannot leak data the caller could not already read.
//!
//! ## The one security-relevant detail
//!
//! Two prompts interpolate a caller-supplied IRI into a SPARQL `IRIREF` (`<…>`). The
//! argument is therefore parsed as an absolute RFC-3987 IRI (`oxrdf::NamedNode::new`)
//! BEFORE it is rendered: such an IRI cannot contain `<`, `>`, `"`, `{`, `}`, `|`, `\`,
//! `^`, a backtick, or any character below `0x21`, so a validated argument provably
//! cannot terminate the `IRIREF` and append clauses of its own. A rejected argument is a
//! JSON-RPC `INVALID_PARAMS` error — never a rendered prompt with the raw text pasted in.

use serde_json::{json, Value};

/// One declared argument of a [`PromptSpec`], as MCP's `PromptArgument`.
pub struct PromptArgument {
    /// The argument name, as it appears as a key of `prompts/get`'s `arguments` object.
    pub name: &'static str,
    /// What the argument means, shown to the user filling the prompt in.
    pub description: &'static str,
    /// Whether `prompts/get` fails without it.
    pub required: bool,
}

impl PromptArgument {
    /// Render this argument as the JSON object MCP's `prompts/list` returns per argument.
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "required": self.required,
        })
    }
}

/// A single MCP prompt advertised in `prompts/list` and rendered by `prompts/get`.
pub struct PromptSpec {
    /// The prompt name a `prompts/get` references.
    pub name: &'static str,
    /// One-line description of what the prompt is for.
    pub description: &'static str,
    /// The arguments the prompt declares (possibly none).
    pub arguments: &'static [PromptArgument],
    /// Render the prompt body from the `prompts/get` `arguments` object. Returns a
    /// caller-facing message on a missing or invalid argument — it never renders a
    /// prompt around an argument it could not validate.
    pub render: fn(&Value) -> Result<String, String>,
}

impl PromptSpec {
    /// Render this spec as the JSON object MCP's `prompts/list` returns per prompt.
    pub fn to_json(&self) -> Value {
        let arguments: Vec<Value> = self.arguments.iter().map(PromptArgument::to_json).collect();
        json!({
            "name": self.name,
            "description": self.description,
            "arguments": arguments,
        })
    }
}

/// A required IRI argument, validated as an absolute RFC-3987 IRI so it is safe to
/// interpolate into a SPARQL `IRIREF`. See the module's security note.
fn iri_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    let raw = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("prompt argument `{}` is required and must be a string", key))?;
    oxrdf::NamedNode::new(raw).map_err(|e| {
        format!(
            "prompt argument `{}` must be an absolute IRI (got `{}`: {})",
            key, raw, e
        )
    })?;
    Ok(raw)
}

/// `explore-dataset`: how to orient in an unfamiliar dataset before writing SPARQL.
pub const EXPLORE_DATASET: PromptSpec = PromptSpec {
    name: "explore-dataset",
    description: "Orient in an unfamiliar RDF dataset: which introspection tools to call, \
                  in what order, and a first query to run.",
    arguments: &[],
    render: |_| {
        Ok("You have MCP access to an RDF dataset served by sparq. Orient yourself \
            before writing SPARQL:\n\
            \n\
            1. Call the `stats` tool for the dataset totals.\n\
            2. Call the `classes` tool for the class IRIs, largest population first.\n\
            3. Call the `prefixes` tool for the namespaces the data actually uses.\n\
            4. Call the `shapes` tool with one class IRI from step 2 to learn the \
            predicates, datatypes and cardinalities that class really has.\n\
            \n\
            Then run SPARQL with the `query` tool. A useful first probe:\n\
            \n\
            SELECT ?class (COUNT(?s) AS ?instances)\n\
            WHERE { ?s a ?class }\n\
            GROUP BY ?class\n\
            ORDER BY DESC(?instances)\n\
            LIMIT 20\n\
            \n\
            Ground every statement in rows the tools actually returned; do not invent \
            IRIs or counts."
            .to_string())
    },
};

/// `count-by-class`: the ready-to-run class census query.
pub const COUNT_BY_CLASS: PromptSpec = PromptSpec {
    name: "count-by-class",
    description: "A ready-to-run SPARQL query counting instances per class, most \
                  populous first.",
    arguments: &[],
    render: |_| {
        Ok("Run this query with the `query` tool and report the result rows:\n\
            \n\
            SELECT ?class (COUNT(?s) AS ?instances)\n\
            WHERE { ?s a ?class }\n\
            GROUP BY ?class\n\
            ORDER BY DESC(?instances)\n\
            \n\
            The `classes` tool answers the same question without SPARQL; prefer it when \
            you only need the counts, and this query when you want to extend it (extra \
            filters, joins, a LIMIT)."
            .to_string())
    },
};

/// `class-overview`: summarise one class, grounded in its shape and instances.
pub const CLASS_OVERVIEW: PromptSpec = PromptSpec {
    name: "class-overview",
    description: "Summarise one class of the dataset, grounded in its data-derived shape \
                  and a sample of real instances.",
    arguments: &[PromptArgument {
        name: "class",
        description: "The class IRI to summarise, e.g. http://xmlns.com/foaf/0.1/Person.",
        required: true,
    }],
    render: |args| {
        let class = iri_arg(args, "class")?;
        Ok(format!(
            "Summarise the class <{class}> of the served RDF dataset.\n\
             \n\
             1. Call the `shapes` tool with {{\"class\": \"{class}\"}} for the predicates, \
             datatypes and cardinalities the data proves for this class.\n\
             2. Run this query with the `query` tool for a sample of real instances:\n\
             \n\
             SELECT ?s ?p ?o\n\
             WHERE {{ ?s a <{class}> . ?s ?p ?o }}\n\
             ORDER BY ?s ?p\n\
             LIMIT 50\n\
             \n\
             Describe only what those two results show. If the shape is empty, say the \
             class has no instances rather than guessing what it would contain.",
            class = class
        ))
    },
};

/// `predicate-usage`: how one predicate is actually used in the data.
pub const PREDICATE_USAGE: PromptSpec = PromptSpec {
    name: "predicate-usage",
    description: "Investigate how one predicate is actually used: how often, and between \
                  what kinds of term.",
    arguments: &[PromptArgument {
        name: "predicate",
        description: "The predicate IRI to investigate, e.g. http://xmlns.com/foaf/0.1/knows.",
        required: true,
    }],
    render: |args| {
        let predicate = iri_arg(args, "predicate")?;
        Ok(format!(
            "Investigate how the predicate <{predicate}> is used in the served RDF \
             dataset.\n\
             \n\
             1. Run this with the `query` tool for the usage count and how many distinct \
             subjects and objects it connects:\n\
             \n\
             SELECT (COUNT(*) AS ?uses) (COUNT(DISTINCT ?s) AS ?subjects) \
             (COUNT(DISTINCT ?o) AS ?objects)\n\
             WHERE {{ ?s <{predicate}> ?o }}\n\
             \n\
             2. Then run this for a sample of real statements:\n\
             \n\
             SELECT ?s ?o\n\
             WHERE {{ ?s <{predicate}> ?o }}\n\
             ORDER BY ?s\n\
             LIMIT 25\n\
             \n\
             Report what the rows show. A zero count means the predicate is unused in \
             this dataset — say so instead of describing what it means elsewhere.",
            predicate = predicate
        ))
    },
};

/// Every prompt this server advertises. Static: the catalog does not depend on the
/// served graph, so `prompts/list` is the same for every dataset.
pub const PROMPTS: &[&PromptSpec] = &[
    &EXPLORE_DATASET,
    &COUNT_BY_CLASS,
    &CLASS_OVERVIEW,
    &PREDICATE_USAGE,
];

/// The advertised prompt with this name, if any.
pub fn find(name: &str) -> Option<&'static PromptSpec> {
    PROMPTS.iter().copied().find(|p| p.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_resolves_every_advertised_prompt_and_nothing_else() {
        for spec in PROMPTS {
            assert_eq!(find(spec.name).map(|s| s.name), Some(spec.name));
        }
        assert!(find("no-such-prompt").is_none());
    }

    #[test]
    fn to_json_carries_name_description_and_declared_arguments() {
        let json = CLASS_OVERVIEW.to_json();
        assert_eq!(json["name"].as_str(), Some("class-overview"));
        assert!(!json["description"].as_str().expect("description").is_empty());
        let args = json["arguments"].as_array().expect("arguments array");
        assert_eq!(args.len(), 1);
        assert_eq!(args[0]["name"].as_str(), Some("class"));
        assert_eq!(args[0]["required"].as_bool(), Some(true));

        // A prompt with no arguments still renders an (empty) array, as MCP expects.
        assert_eq!(
            EXPLORE_DATASET.to_json()["arguments"].as_array().map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn argument_to_json_marks_optionality() {
        let optional = PromptArgument {
            name: "x",
            description: "d",
            required: false,
        };
        assert_eq!(optional.to_json()["required"].as_bool(), Some(false));
    }

    #[test]
    fn argument_free_prompts_render_without_arguments() {
        for spec in [&EXPLORE_DATASET, &COUNT_BY_CLASS] {
            let text = (spec.render)(&json!({})).expect("renders");
            assert!(text.contains("SELECT"), "{} lost its query", spec.name);
        }
    }

    #[test]
    fn class_overview_interpolates_the_validated_iri() {
        let text = (CLASS_OVERVIEW.render)(&json!({"class": "http://ex/Person"}))
            .expect("valid IRI renders");
        assert!(text.contains("<http://ex/Person>"), "{}", text);
    }

    #[test]
    fn predicate_usage_interpolates_the_validated_iri() {
        let text = (PREDICATE_USAGE.render)(&json!({"predicate": "http://ex/knows"}))
            .expect("valid IRI renders");
        assert!(text.contains("<http://ex/knows>"), "{}", text);
    }

    #[test]
    fn missing_required_argument_is_refused() {
        let err = (CLASS_OVERVIEW.render)(&json!({})).expect_err("class is required");
        assert!(err.contains("required"), "{}", err);
        // A non-string value is refused on the same path.
        assert!((CLASS_OVERVIEW.render)(&json!({"class": 7})).is_err());
    }

    /// THE headline guard: an argument that would break out of the SPARQL `IRIREF` it is
    /// interpolated into must be REFUSED, not rendered. Every one of these carries a
    /// character `is_valid_iri` forbids, so none can reach the rendered text.
    #[test]
    fn iri_arguments_that_escape_the_iriref_are_refused() {
        let hostile = [
            // Closes the IRIREF and appends a clause of its own.
            "http://ex/P> . ?x ?y ?z . <http://ex/Q",
            // Opens a second IRIREF.
            "http://ex/<P",
            // Whitespace / newline injection.
            "http://ex/P ?s ?p ?o",
            "http://ex/P\n} INSERT DATA { <http://ex/a> <http://ex/b> <http://ex/c>",
            // Quote and brace terminators.
            "http://ex/P\"",
            "http://ex/P}",
            // Not an absolute IRI at all.
            "Person",
            "",
        ];
        for value in hostile {
            for spec in [&CLASS_OVERVIEW, &PREDICATE_USAGE] {
                let key = spec.arguments[0].name;
                let out = (spec.render)(&json!({ key: value }));
                assert!(
                    out.is_err(),
                    "{} rendered a hostile {} argument {:?}: {:?}",
                    spec.name,
                    key,
                    value,
                    out
                );
            }
        }
    }
}
