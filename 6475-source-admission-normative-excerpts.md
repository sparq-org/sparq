# Normative algorithm text supplied for independent comparison

W3C RDF Dataset Canonicalization, Recommendation21May2024. Copyright ©W3C; reproduced under its permissive document license https://www.w3.org/copyright/document-license/ . Extracted from the frozen public Recommendation HTML. Ordered-list sequence is preserved as paragraphs, but original HTML list numbers are not synthesized; section anchors identify the source. Examples omitted.

## https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#conformance

2. Conformance
As well as sections marked as non-normative, all authoring guidelines, diagrams, examples, and notes in this specification are non-normative. Everything else in this specification is normative.
The key words MUST, MUST NOT, and SHOULD in this document
are to be interpreted as described in
BCP 14
[RFC2119] [RFC8174]
when, and only when, they appear in all capitals, as shown here.
A conforming processor is a system which can generate
the canonical n-quads form of an input dataset
consistent with the algorithms defined in this specification.
The algorithms in this specification are normative,
because to consistently reproduce the same canonical identifiers,
implementations MUST strictly conform to the steps outlined in these algorithms.
Note
Implementers can partially check their level of conformance with
this specification by successfully passing the test cases of the
RDF Dataset Canonicalization test suite.
Note, however, that passing all the tests in the test
suite does not imply complete conformance to this specification. It only implies
that the implementation conforms to the aspects tested by the test suite.

## https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#canon-algo-algo

4.4.3 Algorithm
The following algorithm will run with a minimal number of iterations in each step
for typical input datasets.
In some extreme cases, the algorithm can behave poorly, particularly in Step 5.
Implementations MUST defend against potential denial-of-service attacks
by raising suitable exceptions and terminating early.
See 7.1 Dataset Poisoning for further information.
Note
Implementations can consider placing limits on the number of
calls to 4.8 Hash N-Degree Quads based on the number
of blank nodes in the hash to blank nodes map.
For most typical datasets, more than a couple
of iterations on 4.8 Hash N-Degree Quads per blank node would be unusual.
Create the canonicalization state.
If the input dataset is an N-Quads document,
parse that document into a dataset in the canonicalized dataset,
retaining any blank node identifiers used within that document
in the input blank node identifier map;
otherwise arbitrary identifiers are assigned for each
blank node.
Explanation
This has the effect of initializing the
blank node to quads map,
and the hash to blank nodes map,
as well as instantiating a new canonical issuer.
After this algorithm completes,
the input blank node identifier map state
and canonical issuer may be used to
correlate blank nodes used in the
input dataset with both their original identifiers,
and associated canonical identifiers.
For every quad Q in input dataset:
For each blank node that is a component of Q,
add a reference to Q from the
map entry for the
blank node identifier identifier
in the blank node to quads map,
creating a new entry if necessary,
using the identifier for the blank node found in the
input blank node identifier map.
Explanation
This establishes the blank node to quads map,
relating each blank node with the set of quads
of which it is a component,
via the map for each blank node in the input dataset to its assigned identifier.
Note
Literal components of
quads are not subject to any normalization.
As noted in
Section 3.3
of [RDF11-CONCEPTS],
literal term equality
is based on the
lexical form,
rather than the literal value,
so two literals "01"^^xsd:integer and "1"^^xsd:integer are treated as distinct resources.
Logging
Log the state of the blank node to quads map:
# Blank node to quads map for unique hashes example
ca:
log point: Entering the canonicalization function (4.4.3).
ca.2:
log point: Extract quads for each bnode (4.4.3 (2)).
Bnode to quads:
e0:
- <http://example.com/#p> <http://example.com/#q> _:e0 .
- _:e0 <http://example.com/#s> <http://example.com/#u> .
e1:
- <http://example.com/#p> <http://example.com/#r> _:e1 .
- _:e1 <http://example.com/#t> <http://example.com/#u> .
...
For each key n
in the blank node to quads map:
Explanation
This step creates a hash for every blank node in the input document.
Some blank nodes will lead to a unique hash,
while other blank nodes may share a common hash.
Create a hash, hf(n),
for n according to the
Hash First Degree Quads algorithm.
Append n to the value associated to hf(n) in
hash to blank nodes map,
creating a new entry if necessary.
Logging
Log the results from the Hash First Degree Quads algorithm.
# First degree hashes for unique hashes example
ca:
...
ca.3:
log point: Calculated first degree hashes (4.4.3 (3)).
with:
- identifier: e0
h1dq:
log point: Hash First Degree Quads function (4.6.3).
nquads:
- <http://example.com/#p> <http://example.com/#q> _:a .
- _:a <http://example.com/#s> <http://example.com/#u> .
hash: 21d1dd5ba21f3dee9d76c0c00c260fa6f5d5d65315099e553026f4828d0dc77a
- identifier: e1
h1dq:
log point: Hash First Degree Quads function (4.6.3).
nquads:
- <http://example.com/#p> <http://example.com/#r> _:a .
- _:a <http://example.com/#t> <http://example.com/#u> .
hash: 6fa0b9bdb376852b5743ff39ca4cbf7ea14d34966b2828478fbf222e7c764473
...
For each hash to identifier list
map entry in
hash to blank nodes map, code point ordered by hash:
Explanation
This step establishes the canonical identifier for blank nodes having
a unique hash, which are recorded in the canonical issuer.
If identifier list has more than one entry,
continue to the next mapping.
Use the
Issue Identifier algorithm,
passing canonical issuer and the
single blank node identifier, identifier in
identifier list to issue a
canonical replacement identifier for identifier.
Remove the map entry for hash from the
hash to blank nodes map.
Logging
Log the assigned canonical identifiers.
# Assigned canonical identifiers for shared hashes example
ca:
...
ca.4:
log point: Create canonical replacements for hashes mapping to a single node (4.4.3 (4)).
with:
- identifier: e2
hash: 15973d39de079913dac841ac4fa8c4781c0febfba5e83e5c6e250869587f8659
canonical label: c14n0
- identifier: e3
hash: 7e790a99273eed1dc57e43205d37ce232252c85b26ca4a6ff74ff3b5aea7bccd
canonical label: c14n1
...
For each hash to identifier list
map entry in
hash to blank nodes map, code point ordered by
hash:
Explanation
This step establishes the canonical identifier for blank nodes having
a shared hash.
This is done by creating unique blank node identifiers for all
blank nodes traversed by the Hash N-Degree Quads algorithm,
running through each blank node without a canonical identifier in the order
of the hashes established in the previous step.
Logging
Log hash and identifier list for this iteration.
# Hash and Identifier List for each iteration of step 5 using shared hashes example
ca:
...
ca.5:
log point: Calculate hashes for identifiers with shared hashes (4.4.3 (5)).
with:
- hash: 3b26142829b8887d011d779079a243bd61ab53c3990d550320a17b59ade6ba36
identifier list: [ "e0", "e1"]
...
...
Create hash path list where each item will be a result
of running the
Hash N-Degree Quads algorithm.
Explanation
This list will be populated in step 5.2, and will establish an order for those blank nodes
sharing a common first-degree hash.
For each blank node identifier
n in identifier list:
If a canonical identifier has already been issued for
n, continue to the next
blank node identifier.
Create temporary issuer, an
identifier issuer initialized with the prefix
b.
Use the
Issue Identifier algorithm,
passing temporary issuer and n, to
issue a new temporary blank node identifier bn
to n.
Run the
Hash N-Degree Quads algorithm,
passing the canonicalization state,
n for identifier, and
temporary issuer,
appending the
result to the hash path list.
Logging
Include logs for each call to Hash N-Degree Quads algorithm.
# Logs from calls to Hash N-Degree Quads algorithm for shared hashes example
ca:
...
ca.5:
log point: Calculate hashes for identifiers with shared hashes (4.4.3 (5)).
with:
- hash: 3b26142829b8887d011d779079a243bd61ab53c3990d550320a17b59ade6ba36
identifier list: [ "e0", "e1"]
ca.5.2:
log point: Calculate hashes for identifiers with shared hashes (4.4.3 (5.2)).
with:
- identifier: e0
hndq:
log point: Hash N-Degree Quads function (4.8.3).
identifier: e0
issuer: {e0: b0}
...
...
...
...
For each result in the hash path list,
code point ordered by the hash in result:
Explanation
The previous step created temporary identifiers for the
blank nodes sharing a common first degree hash,
which is now used to generate their canonical identifiers.
For each blank node identifier,
existing identifier, that was issued a temporary
identifier by identifier issuer in result,
issue a canonical identifier,
in the same order,
using the Issue Identifier algorithm,
passing canonical issuer and existing identifier.
Explanation
In Step 5.2,
hash path list was created with an ordered
set of results.
Each result contained a temporary issuer
which recorded temporary identifiers associated with
a particular blank node identifier in
identifier list.
This step processes each returned temporary issuer,
in order, and allocates canonical identifiers
to the temporary identifier mappings contained
within each temporary issuer,
creating a full order on the remaining blank nodes
with unissued canonical identifiers.
Logging
Log newly issued canonical identifiers.
# Newly issued canonical identifiers from step 5.3 for shared hashes example
ca:
...
ca.5:
log point: Calculate hashes for identifiers with shared hashes (4.4.3 (5)).
with:
- hash: 3b26142829b8887d011d779079a243bd61ab53c3990d550320a17b59ade6ba36
identifier list: [ "e0", "e1"]
...
ca.5.3:
log point: Canonical identifiers for temporary identifiers (4.4.3 (5.3)).
issuer:
- blank node: e1
canonical identifier: c14n2
- blank node: e0
canonical identifier: c14n3
...
Add the issued identifiers map
from the canonical issuer to the
canonicalized dataset.
Explanation
This step adds the issued identifiers map
from the canonical issuer to the
canonicalized dataset, the keys in the
issued identifiers map are map entries in the
input blank node identifier map.
Logging
Log the state of the canonical issuer at the completion of the algorithm.
# Canonical issuer state after step 6 for shared hashes example
ca:
...
ca.6:
log point: Issued identifiers map (4.4.3 (6)).
issued identifiers map: {e2: c14n0, e3: c14n1, e1: c14n2, e0: c14n3}
Return the serialized canonical form
of the canonicalized dataset.
Upon request, alternatively (or additionally) return the
canonicalized dataset itself, which includes the
input blank node identifier map, and
issued identifiers map from the canonical issuer.
Note
Technically speaking, one implementation
might return a canonicalized dataset that maps
particular blank nodes to different identifiers than another
implementation, however, this only occurs when there are
isomorphisms in the dataset such that a canonically serialized
expression of the dataset would appear the same from either
implementation.
Explanation
The serialized canonical form is an N-Quads
document where the blank node identifiers are taken
from the canonical identifiers associated with each blank node.
The canonicalized dataset is composed of the original
input dataset, the input blank node identifier map,
containing identifiers for each blank node in the input dataset,
and the canonical issuer,
containing an issued identifiers map
mapping the identifiers in the input blank node identifier map
to their canonical identifiers.

## https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#issue-identifier-algorithm

4.5.2 Algorithm
The algorithm takes an identifier issuer I and an
existing identifier as inputs. The output is a new
issued identifier. The steps of the algorithm are:
If there is a
map entry for existing identifier in
issued identifiers map of I,
return it.
Generate issued identifier by concatenating
identifier prefix with the string value of
identifier counter.
Add an entry
mapping existing identifier to issued identifier
to the issued identifiers map of I.
Increment identifier counter.
Return issued identifier.

## https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#hash-1d-quads-algorithm

4.6.3 Algorithm
This algorithm takes the canonicalization state and a
reference blank node identifier as inputs.
Initialize nquads to an empty list.
It will be used to store quads in canonical n-quads form.
Get the list of quads quads
from the map entry for
reference blank node identifier in the
blank node to quads map.
For each quad quad in quads:
Serialize the quad in canonical n-quads form with the
following special rule:
If any component in quad is an
blank node, then serialize it using a
special identifier as follows:
If the blank node's existing
blank node identifier matches the
reference blank node identifier then use the
blank node identifier a,
otherwise, use the blank node identifier
z.
Sort nquads in Unicode code point order.
Return the hash that results from passing the sorted
and concatenated nquads through the
hash algorithm.
Logging
Log the inputs and result of running this algorithm.
# Inputs and hash result for the Hash First Degree Hash algorithm for unique hashes example
h1dq:
log point: Hash First Degree Quads function (4.6.3).
nquads:
- <http://example.com/#p> <http://example.com/#q> _:a .
- _:a <http://example.com/#s> <http://example.com/#u> .
hash: 21d1dd5ba21f3dee9d76c0c00c260fa6f5d5d65315099e553026f4828d0dc77a

## https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#hash-related-algorithm

4.7.3 Algorithm
This algorithm creates a hash to identify how one
blank node is related to another. It takes the
canonicalization state, a related
blank node identifier, a quad, an
identifier issuer, issuer, and a
string position as inputs.
Initialize a string input to the value of
position.
If position is not g, append
<, the value of the predicate in
quad, and > to input.
If there is a canonical identifier for related,
or an identifier issued by issuer,
append the string _:, followed by that identifier (using the canonical
identifier if present, otherwise the one issued by issuer) to
input.
Explanation
If a canonical identifier was already issued for related,
it will be in the canonical issuer contained within
canonicalization state.
Otherwise, the temporary issuer instance may already
have a mapping for related.
Otherwise,
append the result of the
Hash First Degree Quads algorithm,
passing related to input.
Explanation
If no identifier, canonical or temporary, has already been issued,
a new identifier is created using the
Hash First Degree Quads algorithm.
Note that Hash First Degree Quads algorithm
has already been called on all blank nodes in step 3
of the Canonicalization algorithm.
Implementations should consider reusing a previously determined result rather than execute the
Hash First Degree Quads algorithm again.
Return the hash that results from passing input
through the hash algorithm.
Explanation
This resulting string is used to generate a hash; in this respect, it is similar to
the Hash First Degree Quads algorithm which
uses the serialization of quads in nquads for hashing. For the sake of consistency, the
nquad representation of identifier is used in this step, hence the
appearance of the _: string.
Logging
Log the inputs and result of running this algorithm.
# Inputs and hash result for the Hash Related Blank algorithm for shared hashes example
hndq3.1:
log point: Hash related bnode component (4.8.3 (3.1))
with:
- position: o
related: e2
input: "o<http://example.com/#p>_:c14n0"
hash: 29cf7e22790bc2ed395b81b3933e5329fc7b25390486085cac31ce7252ca60fa

## https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#hash-nd-quads-algorithm

4.8.3 Algorithm
The inputs to this algorithm are the canonicalization state,
the identifier for the blank node to
recursively hash quads for, and path identifier issuer which is
an identifier issuer that issues temporary
blank node identifiers. The output from this algorithm
will be a hash and the identifier issuer used
to help generate it.
Logging
Log the inputs to the algorithm.
# Inputs for the Hash N-Degree Quads algorithm for double circle example
hndq:
log point: Hash N-Degree Quads function (4.8.3).
identifier: e0
issuer: {e0: b0}
...
Create a new map Hn
for relating hashes to related blank nodes.
Get a reference, quads, to the list of quads
from the map entry
for identifier
in the blank node to quads map.
Explanation
quads is the mention set of identifier.
Logging
Log the quads from the mention set of identifier.
# Inputs for the Hash N-Degree Quads algorithm for double circle example
hndq:
identifier: e0
log point: Hash N-Degree Quads function (4.8.3).
issuer: {e0: b0}
hndq.2:
log point: Quads for identifier (4.8.3 (2)).
quads:
- _:e0 <http://example.org/vocab#next> _:e1 .
- _:e0 <http://example.org/vocab#prev> _:e1 .
- _:e1 <http://example.org/vocab#next> _:e0 .
- _:e1 <http://example.org/vocab#prev> _:e0 .
...
For each quad in quads:
Explanation
This loop calculates the related hash Hn
for other blank nodes within the mention set of identifier.
For each component in quad, where component
is the subject, object, or
graph name, and it is a
blank node that is not identified by
identifier:
Set hash to the result of the
Hash Related Blank Node algorithm,
passing the blank node identifier for
component as related, quad,
issuer, and
position as either s, o, or
g based on whether component is a
subject, object,
graph name, respectively.
Add a mapping of hash to the
blank node identifier for component
to Hn, adding an entry
as necessary.
Logging
Include the logs for each iteration of the
Hash Related Blank Node algorithm
and the resulting Hn.
# Step 3 of Hash N-Degree Quads using double circle example
hndq:
identifier: e0
log point: Hash N-Degree Quads function (4.8.3).
issuer: {e0: b0}
...
hndq.3:
log point: Hash N-Degree Quads function (4.8.3 (3)).
with:
- quad: _:e0 <http://example.org/vocab#next> _:e1 .
hndq.3.1:
log point: Hash related bnode component (4.8.3 (3.1))
with:
- position: o
related: e1
h1dq:
log point: Hash First Degree Quads function (4.6.3).
nquads:
- _:z <http://example.org/vocab#next> _:a .
- _:z <http://example.org/vocab#prev> _:a .
- _:a <http://example.org/vocab#next> _:z .
- _:a <http://example.org/vocab#prev> _:z .
hash: 60dc8fc7b5481014b6ea38efb05455676d1e93e19b99119ab294941dacc16b3b
input: "o<http://example.org/vocab#next>60dc8fc7b5481014b6ea38efb05455676d1e93e19b99119ab294941dacc16b3b"
hash: 20bb08971220a5382a9a06ba2977c5fb859e63192e0b2015a378af89e453f25e
- quad: _:e0 <http://example.org/vocab#prev> _:e1 .
...
Hash to bnodes:
20bb08971220a5382a9a06ba2977c5fb859e63192e0b2015a378af89e453f25e:
- e1
1e4e55ba02b8b0b527c32e2343fbcfee2e2bd9c1972c67cc01f85fabde7bc42d:
- e1
56d0774755aaf8d9cf4da8af3728e5589f94e5cd7d9aee86f0c5a7bc1d71c7ca:
- e1
2a5dd448b9467a08479008a5350829441868b7f913343cd500fe8619e047cff4:
- e1
...
Create an empty string, data to hash.
For each related hash to blank node list mapping in
Hn, code point ordered
by related hash:
Explanation
This loop explores the gossip paths for each
related blank node sharing a common hash to identifier
finding the shortest such path (chosen path).
This determines how canonical identifiers for
otherwise commonly hashed blank nodes are chosen.
Each path is represented by the concatenation of the
identifiers for each related blank node
— either the issued identifier,
or a temporary identifier created using a copy of issuer.
Those for which temporary identifiers were issued are later
recursed over using this algorithm.
Logging
Log the value of related hash
and state of data to hash.
# Log related hash and data to hash in each iteration of step 5 for double circle example.
hndq:
log point: Hash N-Degree Quads function (4.8.3).
identifier: e0
issuer: {e0: b0}
...
hndq.5:
log point: Hash N-Degree Quads function (4.8.3 (5)), entering loop.
with:
- related_hash: 1e4e55ba02b8b0b527c32e2343fbcfee2e2bd9c1972c67cc01f85fabde7bc42d
data_to_hash: ""
...
Append the related hash to the data to hash.
Create a string chosen path.
Create an unset chosen issuer variable.
For each permutation p of blank node list:
Logging
Log each permutation p.
# Log each permutation of step 5.4 using double circle example.
hndq:
log point: Hash N-Degree Quads function (4.8.3).
identifier: e0
issuer: {e0: b0}
...
hndq.5:
log point: Hash N-Degree Quads function (4.8.3 (5)), entering loop.
with:
- related_hash: 1e4e55ba02b8b0b527c32e2343fbcfee2e2bd9c1972c67cc01f85fabde7bc42d
data_to_hash: ""
hndq.5.4:
log point: Hash N-Degree Quads function (4.8.3 (5.4)), entering loop.
with:
- perm: [ "e1"]
...
Create a copy of issuer, issuer copy.
Create a string path.
Create a recursion list, to store
blank node identifiers that must be
recursively processed by this algorithm.
For each related in p:
If a canonical identifier has been issued for
related by canonical issuer, append the string _:, followed by
the canonical identifier for related, to path.
Explanation
A canonical identifier may have been generated before calling this algorithm,
if it was issued from an earlier call to Hash First Degree Quads algorithm.
There is no reason to recurse and apply the algorithm to any related blank node that has already been assigned a canonical identifier.
Furthermore, using the canonical identifier also further distinguishes it from any temporary identifier, allowing for even greater efficiency in finding the chosen path.
Otherwise:
If issuer copy has not issued
an identifier for related, append
related to recursion list.
Explanation
Temporarily labeled nodes have identifiers recorded
in issuer copy,
which is later used to recursively call this algorithm,
so that eventually all nodes are given canonical identifiers.
Use the
Issue Identifier algorithm,
passing issuer copy and the related, and
append the string _:, followed by the result, to path.
If chosen path is not empty and the length
of path is greater than or equal to the length
of chosen path and path is
greater than chosen path when
considering code point order,
then skip to the next
permutation p.
Explanation
If path is already longer than
the prospective chosen path,
we can terminate this iteration early.
Explanation
path is used to generate a hash at a later step; in this respect, it is similar to
the Hash First Degree Quads algorithm which
uses the serialization of quads in nquads for hashing. For the sake of consistency, the
nquad representation of blank node identifiers is used in these steps, hence the
usage of the _: string.
Logging
Log related and path.
# Log related and path of step 5.4.4 using double circle example.
hndq:
log point: Hash N-Degree Quads function (4.8.3).
identifier: e0
issuer: {e0: b0}
...
hndq.5:
log point: Hash N-Degree Quads function (4.8.3 (5)), entering loop.
with:
- related_hash: 1e4e55ba02b8b0b527c32e2343fbcfee2e2bd9c1972c67cc01f85fabde7bc42d
data_to_hash: ""
hndq.5.4:
log point: Hash N-Degree Quads function (4.8.3 (5.4)), entering loop.
with:
- perm: [ "e1"]
hndq.5.4.4:
log point: Hash N-Degree Quads function (4.8.3 (5.4.4)), entering loop.
with:
- related: e1
path: ""
...
For each related in recursion list:
Explanation
The prospective path is extended with
the hash resulting from recursively calling this algorithm
on each related blank node issued a temporary identifier.
Logging
Log recursion list and path.
# Log related and path of step 5.4.5 using double circle example.
hndq:
log point: Hash N-Degree Quads function (4.8.3).
identifier: e0
issuer: {e0: b0}
...
hndq.5:
log point: Hash N-Degree Quads function (4.8.3 (5)), entering loop.
with:
- related_hash: 1e4e55ba02b8b0b527c32e2343fbcfee2e2bd9c1972c67cc01f85fabde7bc42d
data_to_hash: ""
hndq.5.4:
log point: Hash N-Degree Quads function (4.8.3 (5.4)), entering loop.
with:
- perm: [ "e1"]
...
hndq.5.4.5:
log point: Hash N-Degree Quads function (4.8.3 (5.4.5)), before possible recursion.
recursion list: [ "e1"]
path: "_:b1"
...
Set result to the result of recursively executing
the Hash N-Degree Quads algorithm,
passing the canonicalization state,
related for identifier, and
issuer copy for path identifier issuer.
Logging
Log related and
include logs for each recursive call to Hash N-Degree Quads algorithm.
# Log related and path of step 5.4.5.1 using double circle example.
hndq:
log point: Hash N-Degree Quads function (4.8.3).
identifier: e0
issuer: {e0: b0}
...
hndq.5:
log point: Hash N-Degree Quads function (4.8.3 (5)), entering loop.
with:
- related_hash: 1e4e55ba02b8b0b527c32e2343fbcfee2e2bd9c1972c67cc01f85fabde7bc42d
data_to_hash: ""
hndq.5.4:
log point: Hash N-Degree Quads function (4.8.3 (5.4)), entering loop.
with:
- perm: [ "e1"]
...
hndq.5.4.5:
log point: Hash N-Degree Quads function (4.8.3 (5.4.5)), before possible recursion.
recursion list: [ "e1"]
path: "_:b1"
with:
- related: e1
hndq:
...
Use the
Issue Identifier algorithm,
passing issuer copy and related; append the string _:, followed by
the result, to path.
Append <, the hash in
result, and > to path.
Set issuer copy to the
identifier issuer in result.
If chosen path is not empty and the length
of path is greater than or equal to the length
of chosen path and path is
greater than chosen path when considering code point order,
then skip to the next p.
Explanation
If path is already longer than
the prospective chosen path,
we can terminate this iteration early.
If chosen path is empty or path is
less than chosen path when considering code point order,
set chosen path to path and chosen issuer
to issuer copy.
Append chosen path to data to hash.
Logging
Log chosen path and data to hash.
# Log chosen path and data to hash logs of step 5.5 using double circle example.
hndq:
log point: Hash N-Degree Quads function (4.8.3).
identifier: e0
issuer: {e0: b0}
...
hndq.5:
log point: Hash N-Degree Quads function (4.8.3 (5)), entering loop.
with:
- related_hash: 1e4e55ba02b8b0b527c32e2343fbcfee2e2bd9c1972c67cc01f85fabde7bc42d
data_to_hash: ""
...
hndq.5.5:
log point: Hash N-Degree Quads function (4.8.3 (5.5). End of current loop with Hn hashes.
chosen path: "_:b1_:b1<1ae899f76e760eb7caf6656437aaef845b50887aff7baeb3531add85ec02ed35>"
data to hash: "1e4e55ba02b8b0b527c32e2343fbcfee2e2bd9c1972c67cc01f85fabde7bc42d_:b1_:b1<1ae899f76e760eb7caf6656437aaef845b50887aff7baeb3531add85ec02ed35>"
...
Replace issuer, by reference, withchosen issuer.
Return issuer and the hash that results from
passing data to hash through the
hash algorithm.
Logging
Log issuer and results from passing data to hash
through the hash algorithm.
# Log issuer and resulting hash of step 6 using double circle example.
hndq:
log point: Hash N-Degree Quads function (4.8.3).
identifier: e0
issuer: {e0: b0}
...
hndq.6:
log point: Leaving Hash N-Degree Quads function (4.8.3).
hash: e332b4b59e1c4794ee72a4df0f63723326ffb6d6a5c0d0cb4d2dd8d8d5ebf5a4
issuer: {e0: b0, e1: b1}

## https://www.w3.org/TR/2024/REC-rdf-canon-20240521/#dataset-poisoning

7.1 Dataset Poisoning
This section is non-normative.
The canonicalization algorithm examines every difference in the
information connected to blank nodes in order to ensure that each will
properly receive its own canonical identifier. This process can be
exploited by attackers to construct datasets which are known to take
large amounts of computing time to canonicalize, but that do not express
useful information or express it using unnecessary complexity.
Implementers of the algorithm are expected to add mitigations that will,
by default, abort canonicalizing problematic inputs.
Suggested mitigations include, but are not limited to:
providing a configurable timeout with a default value applicable to
an implementation's common use
providing a configurable limit on the number of iterations of steps
performed in the algorithm, particularly recursive steps
and permutations of long lists
Additionally, software that uses implementations of the algorithm can
employ best-practice schema validation to reject data that does not meet
application requirements, thereby preventing useless poison datasets from
being processed. However, such mitigations are application specific and
not directly applicable to implementers of the canonicalization algorithm
itself.
