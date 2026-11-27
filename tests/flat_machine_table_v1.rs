#[path = "../src/flat_machine_table_v1.rs"]
mod flat_machine_table_v1;
use flat_machine_table_v1::{Node, Table};

fn node(name: &str, terminal: bool, edges: &[(&str, usize)]) -> Node {
    Node { name: name.into(), terminal, edges: edges.iter().map(|(e,t)| (e.to_string(), *t)).collect() }
}
fn graph() -> Vec<Node> {
    vec![node("Alpha", true, &[]), node("Zed", false, &[("AlphaEvent",0),("Zulu",1)])]
}

#[test]
fn slots_and_successors() {
    let mut input = graph();
    let t = Table::verify(1, &input).unwrap();
    assert_eq!(t.initial(), 1);
    assert_eq!(t.names(), &["Alpha".to_string(),"Zed".to_string()]);
    assert_eq!(t.events(), &["AlphaEvent".to_string(),"Zulu".to_string()]);
    assert_eq!(t.successor(1,0), Ok(Some(0)));
    assert_eq!(t.successor(1,1), Ok(Some(1)));
    assert_eq!(t.successor(0,0), Ok(None));
    assert!(t.successor(2,0).is_err());
    assert!(t.successor(1,2).is_err());
    input[1].edges[0].1 = 1;
    assert_eq!(t.successor(1,0), Ok(Some(0)));
}

#[test]
fn rejects_invalid_graphs() {
    assert!(Table::verify(0,&[]).is_err());
    assert!(Table::verify(2,&graph()).is_err());
    let mut g = graph(); g.reverse(); assert!(Table::verify(1,&g).is_err());
    let mut g = graph(); g[0].edges.push(("X".into(),1)); assert!(Table::verify(1,&g).is_err());
    let mut g = graph(); g[1].edges.reverse(); assert!(Table::verify(1,&g).is_err());
    let mut g = graph(); g[1].edges[0].1=9; assert!(Table::verify(1,&g).is_err());
    let mut g = graph(); g[1].edges.remove(0); assert!(Table::verify(1,&g).is_err());
    assert!(Table::verify(0,&[node("A",false,&[("E",0),("E",0)])]).is_err());
    for invalid in ["", "a\n", "é", "9name", &"a".repeat(1025)] {
        assert!(Table::verify(0,&[node(invalid,true,&[])]).is_err());
    }
}

#[test]
fn exact_limits() {
    let mut g=vec![node("N",false,&[])];
    for i in 0..256 { g[0].edges.push((format!("E{i:03}"),0)); }
    assert_eq!(Table::verify(0,&g).unwrap().events().len(),256);
    g[0].edges.push(("E256".into(),0)); assert!(Table::verify(0,&g).is_err());
    let mut g: Vec<_> = (0..256).map(|i| {
        let edge = [("Next", i + 1)];
        node(&format!("N{i:03}"), i == 255, if i == 255 { &[] } else { &edge })
    }).collect();
    assert_eq!(Table::verify(0,&g).unwrap().names().len(),256);
    g.push(node("N256",true,&[])); assert!(Table::verify(0,&g).is_err());
}
