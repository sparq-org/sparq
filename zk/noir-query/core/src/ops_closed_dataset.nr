//! Lane E stand-in: the signed RDF issuer field supplies the issuer IRI.
//! The issuer gadget binds its signing key through the verifier-owned registry.
use crate::table::{TermCell,Table,unbound,empty,append,canonicalize,check_table,assert_table,same_term,same_row};
use crate::ops_core::Dataset;
use crate::ops_closed_common::get;
pub fn graph_name(credential:u32)->TermCell {
    let c=crate::term::credential_graph([0;2],credential);
    TermCell{bound:c.bound(),kind:c.kind(),enc:c.enc(),key_class:c.key_class(),key:c.key(),dt:c.datatype(),lang:c.language(),scope:c.credential()}
}
pub fn graph_name_request(credential:u32,digest:[u8;32])->TermCell{
    let mut request:[u128;2]=[0;2];for i in 0..32 {request[i/16]=request[i/16]*256+digest[i] as u128;}
    let c=crate::term::credential_graph(request,credential);
    TermCell{bound:c.bound(),kind:c.kind(),enc:c.enc(),key_class:c.key_class(),key:c.key(),dt:c.datatype(),lang:c.language(),scope:c.credential()}
}
pub fn unit_context()->Table<1,1>{append(empty(),[unbound()],true)}
pub fn named<let D:u32>(mut dataset:Dataset<D>,offset:u32,length:u32)->Dataset<D>{
    assert((offset as u64 + length as u64)<=D as u64,"named graph capacity");
    for i in 0..D {dataset.active[i]=dataset.active[i] & (i>=offset) & (i<offset+length);}
    dataset
}
pub fn default_graph<let D:u32,let C:u32,let O:u32>(dataset:Dataset<D>,offsets:[u32;C],lengths:[u32;C],count:u32)->Dataset<O>{
    std::static_assert(O==D+C,"default graph capacity");
    let mut out=Dataset {rows:[[unbound();3];O],active:[false;O],graphs:[unbound();O]};
    for i in 0..D {out.rows[i]=dataset.rows[i];out.active[i]=dataset.active[i];}
    for c in 0..C {
        let mut issuer=unbound();
        for i in 0..D {
            if dataset.active[i] & (i>=offsets[c]) & (i<offsets[c]+lengths[c]) & (dataset.rows[i][1].enc==crate::suite_constants::IRI_ISSUER) {
                let value=dataset.rows[i][2];
                assert((value.kind==1) & (!issuer.bound | same_term(issuer,value)),"credential issuer field");issuer=value;
            }
        }
        if (c<count) & issuer.bound {out.rows[D+c]=[if dataset.graphs[offsets[c]].bound {dataset.graphs[offsets[c]]}else{graph_name(c)},TermCell{bound:true,kind:1,enc:crate::ops_core_suite::VERIFIED_ISSUER_ENCODING,key_class:0,key:0,dt:0,lang:0,scope:0},issuer];out.active[D+c]=true;}
    }
    out
}
pub fn scan<let D:u32,let CR:u32,let VC:u32,let R:u32,let V:u32>(dataset:Dataset<D>,context:Table<CR,VC>,slots:[u32;3],captures:[u32;3],constants:[TermCell;3],out:Table<R,V>){
    check_table(context);assert(context.len<=1,"capture cardinality");
    let mut expected=empty();
    for i in 0..D {
        let mut matches=dataset.active[i] & (context.len>0);
        let mut row=[unbound();V];
        for p in 0..3 {
            assert(slots[p]<=V,"scan column");let outer=get(context.rows[0],captures[p]);
            if outer.bound {matches=matches & same_term(dataset.rows[i][p],outer);}
            else if slots[p]==V {matches=matches & same_term(dataset.rows[i][p],constants[p]);}
            else {for v in 0..V {if slots[p]==v {matches=matches & (!row[v].bound | same_term(row[v],dataset.rows[i][p]));row[v]=dataset.rows[i][p];}}}
        }
        expected=append_match(expected,row,matches);
    }
    assert_table(canonicalize(expected),out);
}
pub fn graph_scan<let D:u32,let C:u32,let CR:u32,let VC:u32,let R:u32,let V:u32>(dataset:Dataset<D>,offsets:[u32;C],lengths:[u32;C],context:Table<CR,VC>,slots:[u32;4],captures:[u32;4],constants:[TermCell;4],out:Table<R,V>){
    let mut named=dataset;
    for c in 0..C {
        let name=graph_name(c);
        for i in 0..D {if (i>=offsets[c]) & (i<offsets[c]+lengths[c]) & !named.graphs[i].bound {named.graphs[i]=name;}}
    }
    graph_scan_bound(named,context,slots,captures,constants,out);
}
/// request_graphs binds every graph cell before this scan is called.
pub fn graph_scan_bound<let D:u32,let CR:u32,let VC:u32,let R:u32,let V:u32>(dataset:Dataset<D>,context:Table<CR,VC>,slots:[u32;4],captures:[u32;4],constants:[TermCell;4],out:Table<R,V>){
    check_table(context);assert(context.len<=1,"capture cardinality");let mut expected=empty();
    for i in 0..D {
        let name=dataset.graphs[i];
        assert(!dataset.active[i] | name.bound,"named scan requires bound graph");
        let quad=[dataset.rows[i][0],dataset.rows[i][1],dataset.rows[i][2],name];
        let mut matches=dataset.active[i] & (context.len>0);let mut row=[unbound();V];
        for p in 0..4 {
            assert(slots[p]<=V,"scan column");let outer=get(context.rows[0],captures[p]);
            if outer.bound {matches=matches & same_term(quad[p],outer);}
            else if slots[p]==V {matches=matches & same_term(quad[p],constants[p]);}
            else {for v in 0..V {if slots[p]==v {matches=matches & (!row[v].bound | same_term(row[v],quad[p]));row[v]=quad[p];}}}
        }
        expected=append_match(expected,row,matches);
    }
    assert_table(canonicalize(expected),out);
}
/// Canonical credential ranks prevent presentation-slot choices from naming graphs.
pub fn canonical_credentials<let C:u32>(messages:[Field;C],count:u32){
    assert(count<=C,"credential count");
    for c in 1..C {assert((c>=count) | !messages[c].lt(messages[c-1]),"canonical credential order");}
}

/// Use the shared suite's request-scoped graph constructor for both fixture and
/// authenticated datasets. Existing authenticated names must agree exactly.
pub fn request_graphs<let D:u32,let C:u32>(mut dataset:Dataset<D>,offsets:[u32;C],lengths:[u32;C],digest:[u8;32])->Dataset<D>{
    let mut request:[u128;2]=[0;2];for i in 0..32 {request[i/16]=request[i/16]*256+digest[i] as u128;}
    for c in 0..C {let a=crate::term::credential_graph(request,c);let name=TermCell{bound:a.bound(),kind:a.kind(),enc:a.enc(),key_class:a.key_class(),key:a.key(),dt:a.datatype(),lang:a.language(),scope:a.credential()};
        for i in 0..D {if (i>=offsets[c]) & (i<offsets[c]+lengths[c]) {
            assert(!dataset.graphs[i].bound | same_term(dataset.graphs[i],name),"credential graph identity");dataset.graphs[i]=name;
        }}
    }
    dataset
}

// For a fixed triple pattern and capture, every unfixed term is represented in
// the result row. Equal matching rows therefore denote the same RDF triple
// (and the same named graph in graph_scan). Deduplicate against the complete
// accumulated result while still visiting every physical source row.
fn append_match<let R:u32,let V:u32>(expected:Table<R,V>,row:[TermCell;V],matches:bool)->Table<R,V>{
    let mut duplicate=false;for j in 0..R {duplicate=duplicate | ((j<expected.len)&same_row(row,expected.rows[j]));}
    append(expected,row,matches & !duplicate)
}
