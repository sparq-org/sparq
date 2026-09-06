audit_select_decoded:
.Lfunc_begin16:
	.cfi_startproc
	pushq	%rbp
	.cfi_def_cfa_offset 16
	pushq	%r15
	.cfi_def_cfa_offset 24
	pushq	%r14
	.cfi_def_cfa_offset 32
	pushq	%r13
	.cfi_def_cfa_offset 40
	pushq	%r12
	.cfi_def_cfa_offset 48
	pushq	%rbx
	.cfi_def_cfa_offset 56
	subq	$40, %rsp
	.cfi_def_cfa_offset 96
	.cfi_offset %rbx, -56
	.cfi_offset %r12, -48
	.cfi_offset %r13, -40
	.cfi_offset %r14, -32
	.cfi_offset %r15, -24
	.cfi_offset %rbp, -16
	movq	%rdx, %rax
	shrq	$61, %rax
	jne	.LBB16_18
	movq	%rdx, %r13
	leaq	(,%rdx,8), %r15
	movabsq	$9223372036854775801, %rax
	cmpq	%rax, %r15
	jae	.LBB16_18
	movq	%rcx, %r12
	movq	%rsi, %r14
	movq	%rdi, %rbx
	testq	%r15, %r15
	movsd	%xmm0, 32(%rsp)
	je	.LBB16_3
	movzbl	__rust_no_alloc_shim_is_unstable(%rip), %eax
	movq	%r15, %rdi
	callq	*malloc@GOTPCREL(%rip)
	movsd	32(%rsp), %xmm0
	movq	%r13, %rcx
	testq	%rax, %rax
	jne	.LBB16_5
	movl	$8, %edi
	movq	%r15, %rsi
	callq	_ZN5alloc5alloc18handle_alloc_error17h29c279d8237d34e5E
.LBB16_3:
	movl	$8, %eax
	xorl	%ecx, %ecx
.LBB16_5:
	movq	%rcx, 8(%rsp)
	movq	%rax, 16(%rsp)
	movq	$0, 24(%rsp)
	testq	%r13, %r13
	je	.LBB16_17
	xorl	%ebp, %ebp
	leaq	.LJTI16_0(%rip), %rcx
	movslq	(%rcx,%r12,4), %r12
	addq	%rcx, %r12
	xorl	%r13d, %r13d
	jmp	.LBB16_7
	.p2align	4
.LBB16_14:
	movq	%r13, (%rax,%rbp,8)
	incq	%rbp
	movq	%rbp, 24(%rsp)
.LBB16_16:
	incq	%r13
	addq	$-8, %r15
	je	.LBB16_17
.LBB16_7:
	movsd	(%r14,%r13,8), %xmm1
	jmpq	*%r12
.LBB16_15:
	ucomisd	%xmm0, %xmm1
	jbe	.LBB16_16
	jmp	.LBB16_12
	.p2align	4
.LBB16_11:
	ucomisd	%xmm0, %xmm1
	jne	.LBB16_16
	jnp	.LBB16_12
	jmp	.LBB16_16
.LBB16_9:
	ucomisd	%xmm1, %xmm0
	jbe	.LBB16_16
	jmp	.LBB16_12
.LBB16_10:
	ucomisd	%xmm1, %xmm0
	jb	.LBB16_16
	jmp	.LBB16_12
.LBB16_8:
	ucomisd	%xmm0, %xmm1
	jb	.LBB16_16
	.p2align	4
.LBB16_12:
	cmpq	8(%rsp), %rbp
	jne	.LBB16_14
	leaq	8(%rsp), %rdi
	leaq	.Lanon.754cce2e2e27f4683415f4b34c48e197.1334(%rip), %rsi
	callq	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h45d93aba1ec97a6aE
	movsd	32(%rsp), %xmm0
	movq	16(%rsp), %rax
	jmp	.LBB16_14
.LBB16_17:
	movq	24(%rsp), %rax
	movq	%rax, 16(%rbx)
	movq	8(%rsp), %rax
	movq	%rax, (%rbx)
	movq	16(%rsp), %rax
	movq	%rax, 8(%rbx)
	movq	%rbx, %rax
	addq	$40, %rsp
	.cfi_def_cfa_offset 56
	popq	%rbx
	.cfi_def_cfa_offset 48
	popq	%r12
	.cfi_def_cfa_offset 40
	popq	%r13
	.cfi_def_cfa_offset 32
	popq	%r14
	.cfi_def_cfa_offset 24
	popq	%r15
	.cfi_def_cfa_offset 16
	popq	%rbp
	.cfi_def_cfa_offset 8
	retq
.LBB16_18:
	.cfi_def_cfa_offset 96
	leaq	.Lanon.754cce2e2e27f4683415f4b34c48e197.1333(%rip), %rdi
	callq	_ZN5alloc7raw_vec17capacity_overflow17h1b4b301db4b7931fE
.Lfunc_end16:
	.size	audit_select_decoded, .Lfunc_end16-audit_select_decoded
