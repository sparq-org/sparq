audit_probe_gather_indices:
.Lfunc_begin20:
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
	subq	$104, %rsp
	.cfi_def_cfa_offset 160
	.cfi_offset %rbx, -56
	.cfi_offset %r12, -48
	.cfi_offset %r13, -40
	.cfi_offset %r14, -32
	.cfi_offset %r15, -24
	.cfi_offset %rbp, -16
	movq	%r8, %rbx
	movq	%rcx, %r12
	movq	%rdx, %r15
	movq	%rsi, %rax
	movq	%rdi, %rcx
	movq	8(%rsi), %rsi
	movq	16(%rax), %rdx
	movq	24(%rdi), %r8
	cmpq	$5, %r8
	jb	.LBB20_2
	movq	8(%rcx), %r8
	movq	16(%rcx), %rcx
	jmp	.LBB20_3
.LBB20_2:
	addq	$4, %rcx
.LBB20_3:
	movabsq	$2611923443488327891, %r13
	movabsq	$1376283091369227076, %r14
	leaq	40(%rsp), %rdi
	callq	_ZN15sparq_substrate4join8JoinKeys9right_key17hf377a0a2ee8113a3E
	movq	40(%rsp), %r10
	movq	56(%rsp), %r11
	cmpq	$2, %r11
	jbe	.LBB20_8
	movq	48(%rsp), %rcx
	leaq	(,%rcx,4), %rax
	cmpq	$5, %rcx
	jae	.LBB20_10
	movq	%r10, %rdx
	cmpq	$1, %rcx
	ja	.LBB20_9
.LBB20_6:
	testq	%rcx, %rcx
	je	.LBB20_13
	movl	(%rdx), %ecx
	movl	-4(%rdx,%rax), %edx
	xorq	%rcx, %r13
	xorq	%rdx, %r14
	movl	$1, %ecx
	jmp	.LBB20_14
.LBB20_8:
	leaq	48(%rsp), %rdx
	leaq	(,%r11,4), %rax
	movq	%r11, %rcx
	cmpq	$1, %rcx
	jbe	.LBB20_6
.LBB20_9:
	xorq	(%rdx), %r13
	xorq	-8(%rdx,%rax), %r14
	jmp	.LBB20_14
.LBB20_10:
	leaq	-1(%rax), %rsi
	movabsq	$-6626703657320631856, %rdi
	movq	%r10, %r9
	.p2align	4
.LBB20_11:
	movq	%r14, %r8
	xorq	(%r9), %r13
	movq	8(%r9), %rdx
	xorq	%rdi, %rdx
	mulxq	%r13, %rdx, %r14
	addq	$-16, %rsi
	xorq	%rdx, %r14
	addq	$16, %r9
	movq	%r8, %r13
	cmpq	$15, %rsi
	ja	.LBB20_11
	xorq	-16(%r10,%rax), %r8
	xorq	-8(%r10,%rax), %r14
	movq	%r8, %r13
	jmp	.LBB20_14
.LBB20_13:
	xorl	%ecx, %ecx
.LBB20_14:
	movq	%r13, %rdx
	mulxq	%r14, %rsi, %rdx
	movabsq	$-1065810590584100411, %rdi
	imulq	%rdi, %rcx
	xorq	%rax, %rsi
	xorq	%rdx, %rsi
	addq	%rcx, %rsi
	imulq	%rdi, %rsi
	rorxq	$38, %rsi, %r13
	cmpq	$1, %r12
	je	.LBB20_17
	movl	%r13d, %edi
	andl	$63, %edi
	cmpq	%r12, %rdi
	jae	.LBB20_63
	shll	$5, %edi
	addq	%rdi, %r15
.LBB20_17:
	movq	%rbx, 32(%rsp)
	movq	%r13, %rax
	shrq	$57, %rax
	movq	(%r15), %r12
	movq	8(%r15), %r8
	vmovd	%eax, %xmm0
	vpbroadcastb	%xmm0, %xmm1
	cmpq	$2, %r11
	jbe	.LBB20_29
	movq	48(%rsp), %rsi
	leaq	(,%rsi,4), %rbp
	xorl	%r9d, %r9d
	vpcmpeqd	%xmm2, %xmm2, %xmm2
.LBB20_19:
	andq	%r8, %r13
	vmovdqu	(%r12,%r13), %xmm3
	vpcmpeqb	%xmm1, %xmm3, %xmm0
	vpmovmskb	%xmm0, %ebx
	testl	%ebx, %ebx
	je	.LBB20_27
	movq	%r8, 24(%rsp)
	vmovdqa	%xmm1, 80(%rsp)
	movq	%rsi, 8(%rsp)
	movq	%r9, (%rsp)
	vmovdqa	%xmm3, 64(%rsp)
.LBB20_21:
	xorl	%r15d, %r15d
	tzcntl	%ebx, %r15d
	addq	%r13, %r15
	andq	%r8, %r15
	negq	%r15
	imulq	$56, %r15, %rcx
	leaq	(%r12,%rcx), %rdi
	movq	-40(%r12,%rcx), %rax
	cmpq	$2, %rax
	movq	%rbx, 16(%rsp)
	jbe	.LBB20_23
	movq	-48(%rdi), %rax
	movq	-56(%r12,%rcx), %rdi
	jmp	.LBB20_24
.LBB20_23:
	addq	$-48, %rdi
.LBB20_24:
	cmpq	%rsi, %rax
	jne	.LBB20_26
	movq	%r10, %rsi
	movq	%rbp, %rdx
	movq	%r10, %rbx
	movq	%r11, %r14
	callq	*bcmp@GOTPCREL(%rip)
	movq	%r14, %r11
	movq	%rbx, %r10
	testl	%eax, %eax
	je	.LBB20_40
.LBB20_26:
	movq	16(%rsp), %rcx
	leal	-1(%rcx), %eax
	andw	%cx, %ax
	movl	%eax, %ebx
	movq	24(%rsp), %r8
	vmovdqa	80(%rsp), %xmm1
	movq	8(%rsp), %rsi
	movq	(%rsp), %r9
	vpcmpeqd	%xmm2, %xmm2, %xmm2
	vmovdqa	64(%rsp), %xmm3
	jne	.LBB20_21
.LBB20_27:
	vpcmpeqb	%xmm2, %xmm3, %xmm0
	vpmovmskb	%xmm0, %eax
	testl	%eax, %eax
	jne	.LBB20_60
	addq	%r9, %r13
	addq	$16, %r13
	addq	$16, %r9
	jmp	.LBB20_19
.LBB20_29:
	leaq	(,%r11,4), %rbp
	leaq	48(%rsp), %rsi
	xorl	%r9d, %r9d
	vpcmpeqd	%xmm2, %xmm2, %xmm2
.LBB20_30:
	andq	%r8, %r13
	vmovdqu	(%r12,%r13), %xmm3
	vpcmpeqb	%xmm1, %xmm3, %xmm0
	vpmovmskb	%xmm0, %ebx
	testl	%ebx, %ebx
	je	.LBB20_38
	movq	%r8, 24(%rsp)
	vmovdqa	%xmm1, 80(%rsp)
	movq	%r9, (%rsp)
	vmovdqa	%xmm3, 64(%rsp)
.LBB20_32:
	xorl	%r15d, %r15d
	tzcntl	%ebx, %r15d
	addq	%r13, %r15
	andq	%r8, %r15
	negq	%r15
	imulq	$56, %r15, %rdi
	leaq	(%r12,%rdi), %rcx
	movq	-40(%r12,%rdi), %rax
	cmpq	$3, %rax
	movq	%rbx, 8(%rsp)
	jb	.LBB20_34
	movq	-56(%r12,%rdi), %rdi
	movq	-48(%rcx), %rax
	jmp	.LBB20_35
.LBB20_34:
	addq	$-48, %rcx
	movq	%rcx, %rdi
.LBB20_35:
	cmpq	%r11, %rax
	jne	.LBB20_37
	movq	%rbp, %rdx
	movq	%r10, %rbx
	movq	%r11, %r14
	movq	%r12, 16(%rsp)
	movq	%rsi, %r12
	callq	*bcmp@GOTPCREL(%rip)
	movq	%r12, %rsi
	movq	16(%rsp), %r12
	movq	%r14, %r11
	movq	%rbx, %r10
	testl	%eax, %eax
	je	.LBB20_40
.LBB20_37:
	movq	8(%rsp), %rcx
	leal	-1(%rcx), %eax
	andw	%cx, %ax
	movl	%eax, %ebx
	movq	24(%rsp), %r8
	vmovdqa	80(%rsp), %xmm1
	movq	(%rsp), %r9
	vpcmpeqd	%xmm2, %xmm2, %xmm2
	vmovdqa	64(%rsp), %xmm3
	jne	.LBB20_32
.LBB20_38:
	vpcmpeqb	%xmm2, %xmm3, %xmm0
	vpmovmskb	%xmm0, %eax
	testl	%eax, %eax
	jne	.LBB20_60
	addq	%r9, %r13
	addq	$16, %r13
	addq	$16, %r9
	jmp	.LBB20_30
.LBB20_40:
	imulq	$56, %r15, %rax
	leaq	(%r12,%rax), %r14
	movq	-8(%r12,%rax), %r15
	cmpq	$3, %r15
	jb	.LBB20_42
	movq	-24(%r14), %r15
	movq	-16(%r14), %r14
	jmp	.LBB20_43
.LBB20_42:
	addq	$-24, %r14
.LBB20_43:
	movq	32(%rsp), %rbx
	movq	(%rbx), %rax
	movq	16(%rbx), %rsi
	subq	%rsi, %rax
	cmpq	%rax, %r15
	ja	.LBB20_47
	testq	%r15, %r15
	je	.LBB20_59
	movq	8(%rbx), %rax
	cmpq	$16, %r15
	jb	.LBB20_46
.LBB20_48:
	leaq	(%rax,%rsi,8), %rcx
	subq	%r14, %rcx
	cmpq	$128, %rcx
	jae	.LBB20_50
.LBB20_46:
	xorl	%ecx, %ecx
.LBB20_53:
	movq	%r15, %rdi
	movq	%rcx, %rdx
	andq	$7, %rdi
	je	.LBB20_55
	.p2align	4
.LBB20_54:
	movq	(%r14,%rdx,8), %r8
	movq	%r8, (%rax,%rsi,8)
	incq	%rsi
	incq	%rdx
	decq	%rdi
	jne	.LBB20_54
.LBB20_55:
	subq	%r15, %rcx
	cmpq	$-8, %rcx
	ja	.LBB20_59
	leaq	(%rax,%rsi,8), %rax
	addq	$56, %rax
	subq	%rdx, %r15
	leaq	(%r14,%rdx,8), %rdx
	addq	$56, %rdx
	xorl	%ecx, %ecx
	.p2align	4
.LBB20_57:
	movq	-56(%rdx,%rcx,8), %rdi
	movq	%rdi, -56(%rax,%rcx,8)
	movq	-48(%rdx,%rcx,8), %rdi
	movq	%rdi, -48(%rax,%rcx,8)
	movq	-40(%rdx,%rcx,8), %rdi
	movq	%rdi, -40(%rax,%rcx,8)
	movq	-32(%rdx,%rcx,8), %rdi
	movq	%rdi, -32(%rax,%rcx,8)
	movq	-24(%rdx,%rcx,8), %rdi
	movq	%rdi, -24(%rax,%rcx,8)
	movq	-16(%rdx,%rcx,8), %rdi
	movq	%rdi, -16(%rax,%rcx,8)
	movq	-8(%rdx,%rcx,8), %rdi
	movq	%rdi, -8(%rax,%rcx,8)
	movq	(%rdx,%rcx,8), %rdi
	movq	%rdi, (%rax,%rcx,8)
	addq	$8, %rcx
	cmpq	%rcx, %r15
	jne	.LBB20_57
	addq	%rcx, %rsi
	jmp	.LBB20_59
.LBB20_50:
	leaq	(,%rsi,8), %rdx
	movq	%r15, %rcx
	andq	$-16, %rcx
	addq	%rax, %rdx
	addq	$96, %rdx
	xorl	%edi, %edi
	.p2align	4
.LBB20_51:
	vmovdqu	(%r14,%rdi,8), %ymm0
	vmovdqu	32(%r14,%rdi,8), %ymm1
	vmovdqu	64(%r14,%rdi,8), %ymm2
	vmovdqu	96(%r14,%rdi,8), %ymm3
	vmovdqu	%ymm0, -96(%rdx,%rdi,8)
	vmovdqu	%ymm1, -64(%rdx,%rdi,8)
	vmovdqu	%ymm2, -32(%rdx,%rdi,8)
	vmovdqu	%ymm3, (%rdx,%rdi,8)
	addq	$16, %rdi
	cmpq	%rdi, %rcx
	jne	.LBB20_51
	addq	%rcx, %rsi
	cmpq	%rcx, %r15
	jne	.LBB20_53
.LBB20_59:
	movq	%rsi, 16(%rbx)
.LBB20_60:
	cmpq	$3, %r11
	jb	.LBB20_62
	movq	%r10, %rdi
	addq	$104, %rsp
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
	jmpq	*free@GOTPCREL(%rip)
.LBB20_62:
	.cfi_def_cfa_offset 160
	addq	$104, %rsp
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
.LBB20_47:
	.cfi_def_cfa_offset 160
	movl	$8, %ecx
	movq	%rbx, %rdi
	movq	%r15, %rdx
	movq	%r10, %r12
	movq	%r11, %r13
	callq	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17he8643a343e567234E
	movq	%r13, %r11
	movq	%r12, %r10
	movq	16(%rbx), %rsi
	movq	8(%rbx), %rax
	cmpq	$16, %r15
	jb	.LBB20_46
	jmp	.LBB20_48
.LBB20_63:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.32(%rip), %rdx
	movq	%r12, %rsi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.Lfunc_end20:
	.size	audit_probe_gather_indices, .Lfunc_end20-audit_probe_gather_indices
