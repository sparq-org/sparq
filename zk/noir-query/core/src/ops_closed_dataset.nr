//! Lane E stand-in: the signed RDF issuer field supplies the issuer IRI.
//! The issuer gadget binds its signing key through the verifier-owned registry.
use crate::table::{TermCell,Table,unbound,empty,append,canonicalize,check_table,assert_table,same_term,same_row};
use crate::ops_core::Dataset;
use crate::ops_closed_common::get;
pub fn graph_name(credential:u32)->TermCell {
    TermCell {bound:true,kind:2,enc:0x0af5528f3405f211743d6e37a79de782ac2e1efe655b142798a59cc5a51c149c,key_class:0,key:0,dt:0,lang:0,scope:credential+1}
}
pub fn unit_context()->Table<1,1>{append(empty(),[unbound()],true)}
pub fn named<let D:u32>(mut dataset:Dataset<D>,offset:u32,length:u32)->Dataset<D>{
    assert((offset as u64 + length as u64)<=D as u64,"named graph capacity");
    for i in 0..D {dataset.active[i]=dataset.active[i] & (i>=offset) & (i<offset+length);}
    dataset
}
pub fn default_graph<let D:u32,let C:u32,let O:u32>(dataset:Dataset<D>,offsets:[u32;C],lengths:[u32;C],count:u32)->Dataset<O>{
    std::static_assert(O==D+C,"default graph capacity");
    let mut out=Dataset {rows:[[unbound();3];O],active:[false;O]};
    for i in 0..D {out.rows[i]=dataset.rows[i];out.active[i]=dataset.active[i];}
    for c in 0..C {
        let mut issuer=unbound();
        for i in 0..D {
            if dataset.active[i] & (i>=offsets[c]) & (i<offsets[c]+lengths[c]) & (dataset.rows[i][1].enc==0x26a71fbee3ab8424550d1b15e6f283d84d8c5430ff6b7b1061fd1bcc1a425849) {
                let value=dataset.rows[i][2];
                assert((value.kind==1) & (!issuer.bound | same_term(issuer,value)),"credential issuer field");issuer=value;
            }
        }
        if (c<count) & issuer.bound {out.rows[D+c]=[graph_name(c),TermCell{bound:true,kind:1,enc:0x11fd433d6cc0a574e28abd5e01ab45ac75e84ec8210d8844aa6d408a266e6ce9,key_class:0,key:0,dt:0,lang:0,scope:0},issuer];out.active[D+c]=true;}
    }
    out
}
pub fn scan<let D:u32,let CR:u32,let VC:u32,let R:u32,let V:u32>(dataset:Dataset<D>,context:Table<CR,VC>,slots:[u32;3],captures:[u32;3],constants:[TermCell;3],out:Table<R,V>){
    check_table(context);assert(context.len<=1,"capture cardinality");
    let mut expected=empty();
    for i in 0..D {
        let mut duplicate=false;
        for j in 0..D {duplicate=duplicate | ((j<i) & dataset.active[j] & same_row(dataset.rows[i],dataset.rows[j]));}
        let mut matches=dataset.active[i] & !duplicate & (context.len>0);
        let mut row=[unbound();V];
        for p in 0..3 {
            assert(slots[p]<=V,"scan column");let outer=get(context.rows[0],captures[p]);
            if outer.bound {matches=matches & same_term(dataset.rows[i][p],outer);}
            else if slots[p]==V {matches=matches & same_term(dataset.rows[i][p],constants[p]);}
            else {for v in 0..V {if slots[p]==v {matches=matches & (!row[v].bound | same_term(row[v],dataset.rows[i][p]));row[v]=dataset.rows[i][p];}}}
        }
        expected=append(expected,row,matches);
    }
    assert_table(canonicalize(expected),out);
}
pub fn graph_scan<let D:u32,let C:u32,let CR:u32,let VC:u32,let R:u32,let V:u32>(dataset:Dataset<D>,offsets:[u32;C],lengths:[u32;C],context:Table<CR,VC>,slots:[u32;4],captures:[u32;4],constants:[TermCell;4],out:Table<R,V>){
    check_table(context);assert(context.len<=1,"capture cardinality");let mut expected=empty();
    for i in 0..D {
        let mut name=unbound();let mut start:u32=0;
        for c in 0..C {if (i>=offsets[c]) & (i<offsets[c]+lengths[c]) {name=graph_name(c);start=offsets[c];}}
        let mut duplicate=false;
        for j in 0..D {duplicate=duplicate | ((j>=start) & (j<i) & dataset.active[j] & same_row(dataset.rows[i],dataset.rows[j]));}
        let quad=[dataset.rows[i][0],dataset.rows[i][1],dataset.rows[i][2],name];
        let mut matches=dataset.active[i] & !duplicate & (context.len>0);let mut row=[unbound();V];
        for p in 0..4 {
            assert(slots[p]<=V,"scan column");let outer=get(context.rows[0],captures[p]);
            if outer.bound {matches=matches & same_term(quad[p],outer);}
            else if slots[p]==V {matches=matches & same_term(quad[p],constants[p]);}
            else {for v in 0..V {if slots[p]==v {matches=matches & (!row[v].bound | same_term(row[v],quad[p]));row[v]=quad[p];}}}
        }
        expected=append(expected,row,matches);
    }
    assert_table(canonicalize(expected),out);
}
/// Canonical credential ranks prevent presentation-slot choices from naming graphs.
pub fn canonical_credentials<let C:u32>(messages:[Field;C],count:u32){
    assert(count<=C,"credential count");
    for c in 1..C {assert((c>=count) | !messages[c].lt(messages[c-1]),"canonical credential order");}
}
