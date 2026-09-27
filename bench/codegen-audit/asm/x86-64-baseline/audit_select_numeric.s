audit_select_numeric:
.Lfunc_begin18:
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
	subq	$56, %rsp
	.cfi_def_cfa_offset 112
	.cfi_offset %rbx, -56
	.cfi_offset %r12, -48
	.cfi_offset %r13, -40
	.cfi_offset %r14, -32
	.cfi_offset %r15, -24
	.cfi_offset %rbp, -16
	movsd	%xmm0, (%rsp)
	movq	%rdx, 48(%rsp)
	cmpq	16(%rsi), %rcx
	jae	.LBB18_1
	movq	%rdi, 40(%rsp)
	movq	8(%rsi), %rax
	leaq	(%rcx,%rcx,2), %rcx
	movq	$0, 8(%rsp)
	movq	$8, 16(%rsp)
	movq	$0, 24(%rsp)
	movq	16(%rax,%rcx,8), %rbp
	testq	%rbp, %rbp
	je	.LBB18_17
	movq	%r8, %r14
	movq	8(%rax,%rcx,8), %r12
	shlq	$2, %rbp
	movl	$8, %eax
	movq	%rax, 32(%rsp)
	xorl	%r13d, %r13d
	leaq	.LJTI18_0(%rip), %rbx
	xorl	%r15d, %r15d
	jmp	.LBB18_4
	.p2align	4
.LBB18_15:
	movq	32(%rsp), %rax
	movq	%r15, (%rax,%r13,8)
	incq	%r13
	movq	%r13, 24(%rsp)
.LBB18_16:
	incq	%r15
	addq	$-4, %rbp
	je	.LBB18_17
.LBB18_4:
	movl	(%r12,%r15,4), %esi
	testl	%esi, %esi
	sets	%cl
	leal	-2147483648(%rsi), %eax
	cmpl	$1073741824, %eax
	setb	%dl
	testb	%dl, %cl
	je	.LBB18_5
	xorps	%xmm0, %xmm0
	cvtsi2sd	%eax, %xmm0
	jmp	.LBB18_7
	.p2align	4
.LBB18_5:
	movq	48(%rsp), %rdi
	callq	_ZN10sparq_core7NumData6lookup17h832debf515382f5bE.702
	testb	$1, %al
	je	.LBB18_16
.LBB18_7:
	movslq	(%rbx,%r14,4), %rax
	addq	%rbx, %rax
	jmpq	*%rax
.LBB18_12:
	ucomisd	(%rsp), %xmm0
	jbe	.LBB18_16
	jmp	.LBB18_13
.LBB18_11:
	ucomisd	(%rsp), %xmm0
	jne	.LBB18_16
	jnp	.LBB18_13
	jmp	.LBB18_16
.LBB18_9:
	movsd	(%rsp), %xmm1
	ucomisd	%xmm0, %xmm1
	jbe	.LBB18_16
	jmp	.LBB18_13
.LBB18_10:
	movsd	(%rsp), %xmm1
	ucomisd	%xmm0, %xmm1
	jb	.LBB18_16
	jmp	.LBB18_13
.LBB18_8:
	ucomisd	(%rsp), %xmm0
	jb	.LBB18_16
	.p2align	4
.LBB18_13:
	cmpq	8(%rsp), %r13
	jne	.LBB18_15
	leaq	8(%rsp), %rdi
	leaq	.Lanon.754cce2e2e27f4683415f4b34c48e197.1332(%rip), %rsi
	callq	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h45d93aba1ec97a6aE
	movq	16(%rsp), %rax
	movq	%rax, 32(%rsp)
	jmp	.LBB18_15
.LBB18_1:
	movq	$0, (%rdi)
	movq	$8, 8(%rdi)
	movq	$0, 16(%rdi)
	jmp	.LBB18_18
.LBB18_17:
	movq	24(%rsp), %rax
	movq	40(%rsp), %rdi
	movq	%rax, 16(%rdi)
	movq	8(%rsp), %rax
	movq	%rax, (%rdi)
	movq	16(%rsp), %rax
	movq	%rax, 8(%rdi)
.LBB18_18:
	movq	%rdi, %rax
	addq	$56, %rsp
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
.Lfunc_end18:
	.size	audit_select_numeric, .Lfunc_end18-audit_select_numeric
