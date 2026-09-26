pragma circom 2.2.2;

// [GPT-6] Unsigned integer units fixed by the issuer-attested schema. This
// research relation is not an implementation of general SPARQL numeric casts.
template Unsigned32() {
    signal input value;
    signal bits[32];
    var reconstructed = 0;
    for (var i = 0; i < 32; i++) {
        bits[i] <-- (value >> i) & 1;
        bits[i] * (bits[i] - 1) === 0;
        reconstructed += bits[i] * (2 ** i);
    }
    value === reconstructed;
}

template Eligibility() {
    // Keep this declaration order: Dock commits the first two private inputs.
    signal input income;
    signal input rent;
    signal input threshold;
    signal residual;
    residual <== income - 12 * rent - threshold;

    component incomeRange = Unsigned32();
    component rentRange = Unsigned32();
    component thresholdRange = Unsigned32();
    component residualRange = Unsigned32();
    incomeRange.value <== income;
    rentRange.value <== rent;
    thresholdRange.value <== threshold;
    residualRange.value <== residual;
}

component main {public [threshold]} = Eligibility();
