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
	subq	$120, %rsp
	.cfi_def_cfa_offset 176
	.cfi_offset %rbx, -56
	.cfi_offset %r12, -48
	.cfi_offset %r13, -40
	.cfi_offset %r14, -32
	.cfi_offset %r15, -24
	.cfi_offset %rbp, -16
	movq	%r8, %r14
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
	movabsq	$1376283091369227076, %rbx
	leaq	56(%rsp), %rdi
	callq	_ZN15sparq_substrate4join8JoinKeys9right_key17hf377a0a2ee8113a3E
	movq	56(%rsp), %r11
	movq	72(%rsp), %rbp
	cmpq	$2, %rbp
	jbe	.LBB20_8
	movq	64(%rsp), %rsi
	leaq	(,%rsi,4), %rcx
	cmpq	$5, %rsi
	jae	.LBB20_10
	movq	%r11, %rax
	movq	%rbx, %rdx
	cmpq	$1, %rsi
	ja	.LBB20_9
.LBB20_6:
	testq	%rsi, %rsi
	je	.LBB20_13
	movl	(%rax), %esi
	movl	-4(%rax,%rcx), %eax
	xorq	%rsi, %r13
	xorq	%rax, %rdx
	movl	$1, %esi
	jmp	.LBB20_14
.LBB20_8:
	leaq	64(%rsp), %rax
	leaq	(,%rbp,4), %rcx
	movq	%rbp, %rsi
	movq	%rbx, %rdx
	cmpq	$1, %rsi
	jbe	.LBB20_6
.LBB20_9:
	xorq	(%rax), %r13
	xorq	-8(%rax,%rcx), %rdx
	jmp	.LBB20_14
.LBB20_10:
	leaq	-1(%rcx), %rdi
	movabsq	$-6626703657320631856, %r8
	movq	%r11, %r10
	movq	%rbx, %rdx
	.p2align	4
.LBB20_11:
	movq	%rdx, %r9
	xorq	(%r10), %r13
	addq	$-16, %rdi
	movq	8(%r10), %rax
	xorq	%r8, %rax
	mulq	%r13
	xorq	%rax, %rdx
	addq	$16, %r10
	movq	%r9, %r13
	cmpq	$15, %rdi
	ja	.LBB20_11
	xorq	-16(%r11,%rcx), %r9
	xorq	-8(%r11,%rcx), %rdx
	movq	%r9, %r13
	jmp	.LBB20_14
.LBB20_13:
	xorl	%esi, %esi
.LBB20_14:
	movabsq	$-1065810590584100411, %rdi
	imulq	%rdi, %rsi
	movq	%r13, %rax
	mulq	%rdx
	xorq	%rcx, %rdx
	xorq	%rax, %rdx
	addq	%rsi, %rdx
	imulq	%rdi, %rdx
	rolq	$26, %rdx
	cmpq	$1, %r12
	je	.LBB20_17
	movl	%edx, %edi
	andl	$63, %edi
	cmpq	%r12, %rdi
	jae	.LBB20_63
	shll	$5, %edi
	addq	%rdi, %r15
.LBB20_17:
	movq	%rdx, %rax
	shrq	$57, %rax
	movq	(%r15), %r13
	movq	8(%r15), %rcx
	movq	%rcx, 8(%rsp)
	movd	%eax, %xmm0
	punpcklbw	%xmm0, %xmm0
	pshuflw	$0, %xmm0, %xmm0
	pshufd	$0, %xmm0, %xmm1
	cmpq	$2, %rbp
	movq	%r14, 48(%rsp)
	jbe	.LBB20_29
	movq	64(%rsp), %rsi
	leaq	(,%rsi,4), %r15
	xorl	%r9d, %r9d
	pcmpeqd	%xmm2, %xmm2
	movq	bcmp@GOTPCREL(%rip), %r12
.LBB20_19:
	andq	8(%rsp), %rdx
	movdqu	(%r13,%rdx), %xmm3
	movdqa	%xmm3, %xmm0
	pcmpeqb	%xmm1, %xmm0
	pmovmskb	%xmm0, %r14d
	testl	%r14d, %r14d
	je	.LBB20_27
	movq	%rdx, 40(%rsp)
	movdqa	%xmm1, 96(%rsp)
	movq	%rsi, 24(%rsp)
	movq	%r9, 16(%rsp)
	movdqa	%xmm3, 80(%rsp)
.LBB20_21:
	rep		bsfl	%r14d, %ebx
	addq	%rdx, %rbx
	andq	8(%rsp), %rbx
	negq	%rbx
	imulq	$56, %rbx, %rcx
	leaq	(%rcx,%r13), %rdi
	movq	-40(%r13,%rcx), %rax
	cmpq	$2, %rax
	movq	%r14, 32(%rsp)
	jbe	.LBB20_23
	movq	-48(%rdi), %rax
	movq	-56(%r13,%rcx), %rdi
	jmp	.LBB20_24
.LBB20_23:
	addq	$-48, %rdi
.LBB20_24:
	cmpq	%rsi, %rax
	jne	.LBB20_26
	movq	%r11, %rsi
	movq	%r15, %rdx
	movq	%rbp, %r14
	movq	%r11, %rbp
	callq	*%r12
	movq	%rbp, %r11
	movq	%r14, %rbp
	testl	%eax, %eax
	je	.LBB20_40
.LBB20_26:
	movq	32(%rsp), %rcx
	leal	-1(%rcx), %eax
	andw	%cx, %ax
	movl	%eax, %r14d
	movq	40(%rsp), %rdx
	movdqa	96(%rsp), %xmm1
	movq	24(%rsp), %rsi
	movq	16(%rsp), %r9
	pcmpeqd	%xmm2, %xmm2
	movdqa	80(%rsp), %xmm3
	jne	.LBB20_21
.LBB20_27:
	pcmpeqb	%xmm2, %xmm3
	pmovmskb	%xmm3, %eax
	testl	%eax, %eax
	jne	.LBB20_60
	addq	%r9, %rdx
	addq	$16, %rdx
	addq	$16, %r9
	jmp	.LBB20_19
.LBB20_29:
	leaq	(,%rbp,4), %r12
	leaq	64(%rsp), %rsi
	xorl	%r9d, %r9d
	pcmpeqd	%xmm2, %xmm2
	movq	bcmp@GOTPCREL(%rip), %r10
.LBB20_30:
	andq	8(%rsp), %rdx
	movdqu	(%r13,%rdx), %xmm3
	movdqa	%xmm3, %xmm0
	pcmpeqb	%xmm1, %xmm0
	pmovmskb	%xmm0, %r14d
	testl	%r14d, %r14d
	je	.LBB20_38
	movq	%rdx, 40(%rsp)
	movdqa	%xmm1, 96(%rsp)
	movq	%r9, 16(%rsp)
	movdqa	%xmm3, 80(%rsp)
.LBB20_32:
	rep		bsfl	%r14d, %ebx
	addq	%rdx, %rbx
	andq	8(%rsp), %rbx
	negq	%rbx
	imulq	$56, %rbx, %rdi
	leaq	(%rdi,%r13), %rcx
	movq	-40(%r13,%rdi), %rax
	cmpq	$3, %rax
	movq	%r14, 24(%rsp)
	jb	.LBB20_34
	movq	-56(%r13,%rdi), %rdi
	movq	-48(%rcx), %rax
	jmp	.LBB20_35
.LBB20_34:
	addq	$-48, %rcx
	movq	%rcx, %rdi
.LBB20_35:
	cmpq	%rbp, %rax
	jne	.LBB20_37
	movq	%r12, %rdx
	movq	%rbp, %r15
	movq	%r11, %rbp
	movq	%r13, 32(%rsp)
	movq	%rsi, %r13
	movq	%r10, %r14
	callq	*%r10
	movq	%r14, %r10
	movq	%r13, %rsi
	movq	32(%rsp), %r13
	movq	%rbp, %r11
	movq	%r15, %rbp
	testl	%eax, %eax
	je	.LBB20_40
.LBB20_37:
	movq	24(%rsp), %rcx
	leal	-1(%rcx), %eax
	andw	%cx, %ax
	movl	%eax, %r14d
	movq	40(%rsp), %rdx
	movdqa	96(%rsp), %xmm1
	movq	16(%rsp), %r9
	pcmpeqd	%xmm2, %xmm2
	movdqa	80(%rsp), %xmm3
	jne	.LBB20_32
.LBB20_38:
	pcmpeqb	%xmm2, %xmm3
	pmovmskb	%xmm3, %eax
	testl	%eax, %eax
	jne	.LBB20_60
	addq	%r9, %rdx
	addq	$16, %rdx
	addq	$16, %r9
	jmp	.LBB20_30
.LBB20_40:
	imulq	$56, %rbx, %rax
	leaq	(%rax,%r13), %r12
	movq	-8(%r13,%rax), %r15
	cmpq	$3, %r15
	jb	.LBB20_42
	movq	-24(%r12), %r15
	movq	-16(%r12), %r12
	jmp	.LBB20_43
.LBB20_42:
	addq	$-24, %r12
.LBB20_43:
	movq	48(%rsp), %rbx
	movq	(%rbx), %rax
	movq	16(%rbx), %rsi
	subq	%rsi, %rax
	cmpq	%rax, %r15
	ja	.LBB20_47
	testq	%r15, %r15
	je	.LBB20_59
	movq	8(%rbx), %rax
	cmpq	$8, %r15
	jb	.LBB20_46
.LBB20_48:
	leaq	(%rax,%rsi,8), %rcx
	subq	%r12, %rcx
	cmpq	$32, %rcx
	jae	.LBB20_50
.LBB20_46:
	xorl	%ecx, %ecx
.LBB20_53:
	movq	%r15, %rdi
	movq	%rcx, %rdx
	andq	$3, %rdi
	je	.LBB20_55
	.p2align	4
.LBB20_54:
	movq	(%r12,%rdx,8), %r8
	movq	%r8, (%rax,%rsi,8)
	incq	%rsi
	incq	%rdx
	decq	%rdi
	jne	.LBB20_54
.LBB20_55:
	subq	%r15, %rcx
	cmpq	$-4, %rcx
	ja	.LBB20_59
	leaq	(%rax,%rsi,8), %rax
	addq	$24, %rax
	subq	%rdx, %r15
	leaq	(%r12,%rdx,8), %rdx
	addq	$24, %rdx
	xorl	%ecx, %ecx
	.p2align	4
.LBB20_57:
	movq	-24(%rdx,%rcx,8), %rdi
	movq	%rdi, -24(%rax,%rcx,8)
	movq	-16(%rdx,%rcx,8), %rdi
	movq	%rdi, -16(%rax,%rcx,8)
	movq	-8(%rdx,%rcx,8), %rdi
	movq	%rdi, -8(%rax,%rcx,8)
	movq	(%rdx,%rcx,8), %rdi
	movq	%rdi, (%rax,%rcx,8)
	addq	$4, %rcx
	cmpq	%rcx, %r15
	jne	.LBB20_57
	addq	%rcx, %rsi
	jmp	.LBB20_59
.LBB20_50:
	leaq	(,%rsi,8), %rdx
	movq	%r15, %rcx
	andq	$-4, %rcx
	addq	%rax, %rdx
	addq	$16, %rdx
	xorl	%edi, %edi
	.p2align	4
.LBB20_51:
	movdqu	(%r12,%rdi,8), %xmm0
	movdqu	16(%r12,%rdi,8), %xmm1
	movdqu	%xmm0, -16(%rdx,%rdi,8)
	movdqu	%xmm1, (%rdx,%rdi,8)
	addq	$4, %rdi
	cmpq	%rdi, %rcx
	jne	.LBB20_51
	addq	%rcx, %rsi
	cmpq	%rcx, %r15
	jne	.LBB20_53
.LBB20_59:
	movq	%rsi, 16(%rbx)
.LBB20_60:
	cmpq	$3, %rbp
	jb	.LBB20_62
	movq	%r11, %rdi
	addq	$120, %rsp
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
	jmpq	*free@GOTPCREL(%rip)
.LBB20_62:
	.cfi_def_cfa_offset 176
	addq	$120, %rsp
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
.LBB20_47:
	.cfi_def_cfa_offset 176
	movl	$8, %ecx
	movq	%rbx, %rdi
	movq	%r15, %rdx
	movq	%r11, %r14
	callq	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17he8643a343e567234E
	movq	%r14, %r11
	movq	16(%rbx), %rsi
	movq	8(%rbx), %rax
	cmpq	$8, %r15
	jb	.LBB20_46
	jmp	.LBB20_48
.LBB20_63:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.32(%rip), %rdx
	movq	%r12, %rsi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.Lfunc_end20:
	.size	audit_probe_gather_indices, .Lfunc_end20-audit_probe_gather_indices
