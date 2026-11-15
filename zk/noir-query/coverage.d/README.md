Expression coverage fragments

Each lane fragment is a JSON object with feature, function and case arrays using the
central manifest schema. Entries replace matching central rows by identifier. Passing
cases run through the shared engine, interpreter and generated-circuit runner; a
fragment cannot establish coverage through host evaluation alone.
