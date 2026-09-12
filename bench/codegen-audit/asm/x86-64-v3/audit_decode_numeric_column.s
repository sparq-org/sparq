audit_decode_numeric_column:
.Lfunc_begin17:
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
	pushq	%rax
	.cfi_def_cfa_offset 64
	.cfi_offset %rbx, -56
	.cfi_offset %r12, -48
	.cfi_offset %r13, -40
	.cfi_offset %r14, -32
	.cfi_offset %r15, -24
	.cfi_offset %rbp, -16
	movq	%rdi, %rbx
	cmpq	16(%rsi), %rcx
	jae	.LBB17_14
	movq	%rdx, %r15
	movq	8(%rsi), %rdx
	movabsq	$9223372036854775800, %rax
	leaq	(%rcx,%rcx,2), %rcx
	movq	8(%rdx,%rcx,8), %rbp
	movq	16(%rdx,%rcx,8), %r13
	leaq	(,%r13,4), %r12
	addq	%rbp, %r12
	leaq	(,%r13,4), %rcx
	xorl	%edx, %edx
	.p2align	4
.LBB17_2:
	cmpq	%rdx, %rcx
	je	.LBB17_15
	cmpl	$-1073741824, (%rbp,%rdx)
	leaq	4(%rdx), %rdx
	jl	.LBB17_2
	leaq	(,%r13,8), %r12
	cmpq	%rax, %r12
	ja	.LBB17_45
	testq	%r12, %r12
	je	.LBB17_24
	movzbl	__rust_no_alloc_shim_is_unstable(%rip), %eax
	movq	%r12, %rdi
	callq	*malloc@GOTPCREL(%rip)
	movq	%rax, %r14
	movq	%r13, (%rsp)
	testq	%rax, %rax
	je	.LBB17_47
	testq	%r13, %r13
	je	.LBB17_25
.LBB17_8:
	xorl	%r12d, %r12d
	jmp	.LBB17_11
	.p2align	4
.LBB17_9:
	vcvtsi2sd	%eax, %xmm1, %xmm0
.LBB17_10:
	vmovsd	%xmm0, (%r14,%r12,8)
	incq	%r12
	cmpq	%r12, %r13
	je	.LBB17_21
.LBB17_11:
	movl	(%rbp,%r12,4), %esi
	testl	%esi, %esi
	sets	%cl
	leal	-2147483648(%rsi), %eax
	cmpl	$1073741824, %eax
	setb	%dl
	testb	%dl, %cl
	jne	.LBB17_9
	movq	%r15, %rdi
	callq	_ZN10sparq_core7NumData6lookup17h832debf515382f5bE.697
	testb	$1, %al
	jne	.LBB17_10
	vmovsd	.LCPI17_0(%rip), %xmm0
	jmp	.LBB17_10
.LBB17_14:
	movl	$8, %r14d
	xorl	%r8d, %r8d
	xorl	%r13d, %r13d
	jmp	.LBB17_44
.LBB17_15:
	leaq	(,%r13,8), %r15
	cmpq	%rax, %r15
	ja	.LBB17_45
	testq	%r15, %r15
	je	.LBB17_22
	movzbl	__rust_no_alloc_shim_is_unstable(%rip), %eax
	movq	%r15, %rdi
	callq	*malloc@GOTPCREL(%rip)
	movq	%rax, %r14
	movq	%r13, %r8
	testq	%rax, %rax
	je	.LBB17_46
	testq	%r13, %r13
	je	.LBB17_23
.LBB17_19:
	cmpq	$4, %r13
	jb	.LBB17_20
	cmpq	%r12, %r14
	jae	.LBB17_29
	addq	%r14, %r15
	cmpq	%r15, %rbp
	jae	.LBB17_29
.LBB17_20:
	xorl	%eax, %eax
.LBB17_38:
	movl	%r13d, %edx
	subl	%eax, %edx
	movq	%rax, %rcx
	andl	$7, %edx
	je	.LBB17_41
	movl	$-2147483648, %esi
	movq	%rax, %rcx
	.p2align	4
.LBB17_40:
	movl	(%rbp,%rcx,4), %edi
	xorl	%esi, %edi
	vcvtsi2sd	%rdi, %xmm6, %xmm0
	vmovsd	%xmm0, (%r14,%rcx,8)
	incq	%rcx
	decq	%rdx
	jne	.LBB17_40
.LBB17_41:
	subq	%r13, %rax
	cmpq	$-8, %rax
	ja	.LBB17_44
	movl	$-2147483648, %eax
	.p2align	4
.LBB17_43:
	movl	(%rbp,%rcx,4), %edx
	xorl	%eax, %edx
	vcvtsi2sd	%rdx, %xmm6, %xmm0
	vmovsd	%xmm0, (%r14,%rcx,8)
	movl	4(%rbp,%rcx,4), %edx
	xorl	%eax, %edx
	vcvtsi2sd	%rdx, %xmm6, %xmm0
	vmovsd	%xmm0, 8(%r14,%rcx,8)
	movl	8(%rbp,%rcx,4), %edx
	xorl	%eax, %edx
	vcvtsi2sd	%rdx, %xmm6, %xmm0
	vmovsd	%xmm0, 16(%r14,%rcx,8)
	movl	12(%rbp,%rcx,4), %edx
	xorl	%eax, %edx
	vcvtsi2sd	%rdx, %xmm6, %xmm0
	vmovsd	%xmm0, 24(%r14,%rcx,8)
	movl	16(%rbp,%rcx,4), %edx
	xorl	%eax, %edx
	vcvtsi2sd	%rdx, %xmm6, %xmm0
	vmovsd	%xmm0, 32(%r14,%rcx,8)
	movl	20(%rbp,%rcx,4), %edx
	xorl	%eax, %edx
	vcvtsi2sd	%rdx, %xmm6, %xmm0
	vmovsd	%xmm0, 40(%r14,%rcx,8)
	movl	24(%rbp,%rcx,4), %edx
	xorl	%eax, %edx
	vcvtsi2sd	%rdx, %xmm6, %xmm0
	vmovsd	%xmm0, 48(%r14,%rcx,8)
	movl	28(%rbp,%rcx,4), %edx
	xorl	%eax, %edx
	vcvtsi2sd	%rdx, %xmm6, %xmm0
	vmovsd	%xmm0, 56(%r14,%rcx,8)
	addq	$8, %rcx
	cmpq	%rcx, %r13
	jne	.LBB17_43
	jmp	.LBB17_44
.LBB17_21:
	movq	(%rsp), %r8
	jmp	.LBB17_44
.LBB17_22:
	movl	$8, %r14d
	xorl	%r8d, %r8d
	testq	%r13, %r13
	jne	.LBB17_19
.LBB17_23:
	xorl	%r13d, %r13d
	jmp	.LBB17_44
.LBB17_24:
	movl	$8, %r14d
	movq	$0, (%rsp)
	testq	%r13, %r13
	jne	.LBB17_8
.LBB17_25:
	xorl	%r13d, %r13d
	movq	(%rsp), %r8
.LBB17_44:
	movq	%r8, (%rbx)
	movq	%r14, 8(%rbx)
	movq	%r13, 16(%rbx)
	movq	%rbx, %rax
	addq	$8, %rsp
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
	vzeroupper
	retq
.LBB17_29:
	.cfi_def_cfa_offset 64
	cmpq	$16, %r13
	jae	.LBB17_31
	xorl	%eax, %eax
	jmp	.LBB17_35
.LBB17_31:
	movq	%r13, %rax
	andq	$-16, %rax
	xorl	%ecx, %ecx
	vpbroadcastd	.LCPI17_1(%rip), %xmm0
	vpbroadcastq	.LCPI17_2(%rip), %ymm1
	.p2align	4
.LBB17_32:
	vpxor	(%rbp,%rcx,4), %xmm0, %xmm2
	vpxor	16(%rbp,%rcx,4), %xmm0, %xmm3
	vpxor	32(%rbp,%rcx,4), %xmm0, %xmm4
	vpxor	48(%rbp,%rcx,4), %xmm0, %xmm5
	vpmovzxdq	%xmm2, %ymm2
	vpor	%ymm1, %ymm2, %ymm2
	vsubpd	%ymm1, %ymm2, %ymm2
	vpmovzxdq	%xmm3, %ymm3
	vpor	%ymm1, %ymm3, %ymm3
	vsubpd	%ymm1, %ymm3, %ymm3
	vpmovzxdq	%xmm4, %ymm4
	vpor	%ymm1, %ymm4, %ymm4
	vsubpd	%ymm1, %ymm4, %ymm4
	vpmovzxdq	%xmm5, %ymm5
	vpor	%ymm1, %ymm5, %ymm5
	vsubpd	%ymm1, %ymm5, %ymm5
	vmovupd	%ymm2, (%r14,%rcx,8)
	vmovupd	%ymm3, 32(%r14,%rcx,8)
	vmovupd	%ymm4, 64(%r14,%rcx,8)
	vmovupd	%ymm5, 96(%r14,%rcx,8)
	addq	$16, %rcx
	cmpq	%rcx, %rax
	jne	.LBB17_32
	cmpq	%rax, %r13
	je	.LBB17_44
	testb	$12, %r13b
	je	.LBB17_38
.LBB17_35:
	movq	%rax, %rcx
	movq	%r13, %rax
	andq	$-4, %rax
	vpbroadcastd	.LCPI17_1(%rip), %xmm0
	vpbroadcastq	.LCPI17_2(%rip), %ymm1
	.p2align	4
.LBB17_36:
	vpxor	(%rbp,%rcx,4), %xmm0, %xmm2
	vpmovzxdq	%xmm2, %ymm2
	vpor	%ymm1, %ymm2, %ymm2
	vsubpd	%ymm1, %ymm2, %ymm2
	vmovupd	%ymm2, (%r14,%rcx,8)
	addq	$4, %rcx
	cmpq	%rcx, %rax
	jne	.LBB17_36
	cmpq	%rax, %r13
	je	.LBB17_44
	jmp	.LBB17_38
.LBB17_45:
	leaq	.Lanon.9ba542688b8e296d0080271b2a3eb5fb.31(%rip), %rdi
	callq	_ZN5alloc7raw_vec17capacity_overflow17h1b4b301db4b7931fE
.LBB17_46:
	movl	$8, %edi
	movq	%r15, %rsi
	callq	_ZN5alloc5alloc18handle_alloc_error17h29c279d8237d34e5E
.LBB17_47:
	movl	$8, %edi
	movq	%r12, %rsi
	callq	_ZN5alloc5alloc18handle_alloc_error17h29c279d8237d34e5E
.Lfunc_end17:
	.size	audit_decode_numeric_column, .Lfunc_end17-audit_decode_numeric_column
