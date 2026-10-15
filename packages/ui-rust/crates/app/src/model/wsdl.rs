//! A WSDL as the SOAP request's WSDL tab uses it: the operations a service offers, and an envelope to start one.
//!
//! [`parse`] reads the raw WSDL text `soap.fetchWsdl` returns. Nothing here knows about a window.
//! shortcut: the envelope has no argument elements (they live in the XSD, which is not read) and its namespace
//! is the WSDL's target namespace, right for document/literal wrappers but not for every RPC style; upgrade
//! by reading the schema when generated arguments are wanted.

use roxmltree::{Document, Node};

const WSDL: &str = "http://schemas.xmlsoap.org/wsdl/";
const SOAP11_BINDING: &str = "http://schemas.xmlsoap.org/wsdl/soap/";
const SOAP12_BINDING: &str = "http://schemas.xmlsoap.org/wsdl/soap12/";
const SOAP11_ENVELOPE: &str = "http://schemas.xmlsoap.org/soap/envelope/";
const SOAP12_ENVELOPE: &str = "http://www.w3.org/2003/05/soap-envelope";

#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    pub name: String,
    pub action: String,
    /// SOAP 1.2 when the binding is a `soap12` one, else 1.1.
    pub soap12: bool,
    /// The `soap:address` of the first port using this operation's binding; empty when the WSDL lists none.
    pub endpoint: String,
    pub namespace: String,
}

impl Operation {
    pub fn version(&self) -> &'static str {
        if self.soap12 { "1.2" } else { "1.1" }
    }
}

fn local(qualified: &str) -> &str {
    qualified.rsplit(':').next().unwrap_or(qualified)
}

fn is(node: &Node, namespaces: &[&str], name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && node.tag_name().namespace().is_some_and(|ns| namespaces.contains(&ns))
}

fn child<'a>(node: Node<'a, 'a>, namespace: &str, name: &str) -> Option<Node<'a, 'a>> {
    node.children().find(|c| is(c, &[namespace], name))
}

/// Every operation of every SOAP binding, in document order. Bindings of other kinds (HTTP) are left out.
pub fn parse(text: &str) -> Result<Vec<Operation>, String> {
    let doc = Document::parse(text).map_err(|error| error.to_string())?;
    let root = doc.root_element();
    if !is(&root, &[WSDL], "definitions") {
        return Err("not a WSDL document".into());
    }
    let namespace = root.attribute("targetNamespace").unwrap_or_default();
    let address_of = |binding: &str| {
        root.descendants()
            .filter(|n| is(n, &[WSDL], "port") && n.attribute("binding").map(local) == Some(binding))
            .find_map(|port| port.children().find(|c| is(c, &[SOAP11_BINDING, SOAP12_BINDING], "address")))
            .and_then(|address| address.attribute("location"))
            .unwrap_or_default()
    };
    let mut operations = Vec::new();
    for binding in root.children().filter(|n| is(n, &[WSDL], "binding")) {
        let soap12 = if child(binding, SOAP12_BINDING, "binding").is_some() {
            true
        } else if child(binding, SOAP11_BINDING, "binding").is_some() {
            false
        } else {
            continue;
        };
        let version = if soap12 { SOAP12_BINDING } else { SOAP11_BINDING };
        let endpoint = address_of(binding.attribute("name").unwrap_or_default());
        for operation in binding.children().filter(|n| is(n, &[WSDL], "operation")) {
            let Some(name) = operation.attribute("name") else { continue };
            let action = child(operation, version, "operation").and_then(|o| o.attribute("soapAction")).unwrap_or_default();
            operations.push(Operation {
                name: name.to_owned(),
                action: action.to_owned(),
                soap12,
                endpoint: endpoint.to_owned(),
                namespace: namespace.to_owned(),
            });
        }
    }
    Ok(operations)
}

/// An envelope with an empty element for the operation in its body.
pub fn skeleton(operation: &Operation) -> String {
    let envelope = if operation.soap12 { SOAP12_ENVELOPE } else { SOAP11_ENVELOPE };
    format!(
        "<soap:Envelope xmlns:soap=\"{envelope}\" xmlns:ns=\"{}\">\n  <soap:Body>\n    <ns:{}/>\n  </soap:Body>\n</soap:Envelope>",
        operation.namespace, operation.name
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOAP11: &str = r#"<?xml version="1.0"?>
<wsdl:definitions xmlns:wsdl="http://schemas.xmlsoap.org/wsdl/" xmlns:soap="http://schemas.xmlsoap.org/wsdl/soap/"
    xmlns:tns="urn:calc" targetNamespace="urn:calc">
  <wsdl:binding name="CalcSoap" type="tns:CalcPort">
    <soap:binding style="document" transport="http://schemas.xmlsoap.org/soap/http"/>
    <wsdl:operation name="Add">
      <soap:operation soapAction="urn:calc/Add"/>
    </wsdl:operation>
  </wsdl:binding>
  <wsdl:service name="Calc">
    <wsdl:port name="CalcSoap" binding="tns:CalcSoap"><soap:address location="http://h/calc"/></wsdl:port>
  </wsdl:service>
</wsdl:definitions>"#;

    const SOAP12: &str = r#"<definitions xmlns="http://schemas.xmlsoap.org/wsdl/" xmlns:soap12="http://schemas.xmlsoap.org/wsdl/soap12/"
    xmlns:tns="urn:t" targetNamespace="urn:t">
  <binding name="B12" type="tns:P">
    <soap12:binding transport="http://schemas.xmlsoap.org/soap/http"/>
    <operation name="Ping"><soap12:operation soapAction="ping"/></operation>
  </binding>
  <service name="S"><port name="p" binding="tns:B12"><soap12:address location="https://h/s12"/></port></service>
</definitions>"#;

    const MULTI: &str = r#"<definitions xmlns="http://schemas.xmlsoap.org/wsdl/" xmlns:soap="http://schemas.xmlsoap.org/wsdl/soap/"
    xmlns:http="http://schemas.xmlsoap.org/wsdl/http/" targetNamespace="urn:m">
  <binding name="A" type="P"><soap:binding/>
    <operation name="One"><soap:operation soapAction="a1"/></operation>
    <operation name="Two"><soap:operation soapAction=""/></operation>
  </binding>
  <binding name="H" type="P"><http:binding verb="GET"/><operation name="Skipped"/></binding>
  <service name="S"><port name="p" binding="A"><soap:address location="http://m/a"/></port></service>
</definitions>"#;

    #[test]
    fn reads_a_soap_11_operation() {
        let ops = parse(SOAP11).unwrap();
        assert_eq!(
            ops,
            vec![Operation {
                name: "Add".into(),
                action: "urn:calc/Add".into(),
                soap12: false,
                endpoint: "http://h/calc".into(),
                namespace: "urn:calc".into()
            }]
        );
        assert_eq!(ops[0].version(), "1.1");
    }

    #[test]
    fn reads_a_soap_12_operation() {
        let ops = parse(SOAP12).unwrap();
        assert_eq!(ops.len(), 1);
        assert!(ops[0].soap12);
        assert_eq!((ops[0].action.as_str(), ops[0].endpoint.as_str()), ("ping", "https://h/s12"));
    }

    #[test]
    fn reads_every_operation_and_skips_non_soap_bindings() {
        let ops = parse(MULTI).unwrap();
        let names: Vec<_> = ops.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["One", "Two"]);
        assert_eq!(ops[1].action, "");
        assert!(ops.iter().all(|o| o.endpoint == "http://m/a"));
    }

    #[test]
    fn rejects_text_that_is_not_a_wsdl() {
        assert!(parse("<html/>").is_err());
        assert!(parse("nope").is_err());
    }

    #[test]
    fn skeleton_names_the_operation_and_the_envelope_version() {
        let ops = parse(SOAP11).unwrap();
        let text = skeleton(&ops[0]);
        assert!(text.contains(r#"xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/""#));
        assert!(text.contains(r#"xmlns:ns="urn:calc""#));
        assert!(text.contains("<ns:Add/>"));
        assert!(skeleton(&parse(SOAP12).unwrap()[0]).contains("http://www.w3.org/2003/05/soap-envelope"));
        assert!(roxmltree::Document::parse(&text).is_ok());
    }
}
