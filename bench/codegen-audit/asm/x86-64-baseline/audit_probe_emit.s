audit_probe_emit:
.Lfunc_begin19:
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
	subq	$248, %rsp
	.cfi_def_cfa_offset 304
	.cfi_offset %rbx, -56
	.cfi_offset %r12, -48
	.cfi_offset %r13, -40
	.cfi_offset %r14, -32
	.cfi_offset %r15, -24
	.cfi_offset %rbp, -16
	movq	%r9, %r14
	movq	%r8, %rbx
	movq	%rcx, 160(%rsp)
	movq	%rdx, 216(%rsp)
	movq	%rsi, %rax
	movq	%rdi, %r12
	movq	8(%rsi), %rsi
	movq	16(%rax), %rdx
	movq	24(%rdi), %r15
	cmpq	$5, %r15
	jb	.LBB19_2
	movq	8(%r12), %r8
	movq	16(%r12), %rcx
	jmp	.LBB19_3
.LBB19_2:
	leaq	4(%r12), %rcx
	movq	%r15, %r8
.LBB19_3:
	movq	320(%rsp), %rax
	movq	%rax, 32(%rsp)
	movq	312(%rsp), %rax
	movq	%rax, 88(%rsp)
	movq	304(%rsp), %rax
	movq	%rax, 168(%rsp)
	leaq	136(%rsp), %rdi
	callq	_ZN15sparq_substrate4join8JoinKeys9right_key17hf377a0a2ee8113a3E
	movq	136(%rsp), %r9
	movq	152(%rsp), %r13
	cmpq	$2, %r13
	jbe	.LBB19_4
	movq	144(%rsp), %rdi
	leaq	(,%rdi,4), %rsi
	cmpq	$5, %rdi
	jae	.LBB19_10
	movq	%r9, %rax
	movabsq	$2611923443488327891, %rcx
	movabsq	$1376283091369227076, %rdx
	cmpq	$1, %rdi
	ja	.LBB19_14
.LBB19_8:
	testq	%rdi, %rdi
	je	.LBB19_9
	movl	(%rax), %edi
	movl	-4(%rax,%rsi), %eax
	xorq	%rdi, %rcx
	xorq	%rax, %rdx
	movl	$1, %edi
	jmp	.LBB19_15
.LBB19_4:
	leaq	144(%rsp), %rax
	leaq	(,%r13,4), %rsi
	movq	%r13, %rdi
	movabsq	$2611923443488327891, %rcx
	movabsq	$1376283091369227076, %rdx
	cmpq	$1, %rdi
	jbe	.LBB19_8
.LBB19_14:
	xorq	(%rax), %rcx
	xorq	-8(%rax,%rsi), %rdx
	jmp	.LBB19_15
.LBB19_10:
	movq	%r14, %rbp
	movabsq	$2611923443488327891, %r11
	movabsq	$1376283091369227076, %rdx
	leaq	-1(%rsi), %r8
	movq	%r9, %r10
	movabsq	$-6626703657320631856, %r9
	movq	%r10, %r14
	.p2align	4
.LBB19_11:
	movq	%rdx, %rcx
	xorq	(%r10), %r11
	addq	$-16, %r8
	movq	8(%r10), %rax
	xorq	%r9, %rax
	mulq	%r11
	xorq	%rax, %rdx
	addq	$16, %r10
	movq	%rcx, %r11
	cmpq	$15, %r8
	ja	.LBB19_11
	movq	%r14, %r9
	xorq	-16(%r14,%rsi), %rcx
	xorq	-8(%r14,%rsi), %rdx
	movq	%rbp, %r14
	jmp	.LBB19_15
.LBB19_9:
	xorl	%edi, %edi
.LBB19_15:
	movabsq	$-1065810590584100411, %r8
	imulq	%r8, %rdi
	movq	%rcx, %rax
	mulq	%rdx
	movq	%rdx, %rbp
	xorq	%rsi, %rbp
	xorq	%rax, %rbp
	addq	%rdi, %rbp
	imulq	%r8, %rbp
	rolq	$26, %rbp
	cmpq	$1, %r14
	je	.LBB19_18
	movl	%ebp, %edi
	andl	$63, %edi
	cmpq	%r14, %rdi
	jae	.LBB19_41
	shll	$5, %edi
	addq	%rdi, %rbx
.LBB19_18:
	movq	%rbp, %rax
	shrq	$57, %rax
	movq	(%rbx), %r8
	movq	8(%rbx), %rcx
	movd	%eax, %xmm0
	punpcklbw	%xmm0, %xmm0
	pshuflw	$0, %xmm0, %xmm0
	pshufd	$0, %xmm0, %xmm1
	cmpq	$2, %r13
	jbe	.LBB19_19
	movq	144(%rsp), %rsi
	leaq	(,%rsi,4), %rdx
	xorl	%edi, %edi
	pcmpeqd	%xmm2, %xmm2
	movq	bcmp@GOTPCREL(%rip), %r14
.LBB19_25:
	andq	%rcx, %rbp
	movdqu	(%r8,%rbp), %xmm3
	movdqa	%xmm3, %xmm0
	pcmpeqb	%xmm1, %xmm0
	pmovmskb	%xmm0, %r10d
	testl	%r10d, %r10d
	je	.LBB19_33
	movq	%rcx, 40(%rsp)
	movdqa	%xmm1, 192(%rsp)
	movq	%rsi, 64(%rsp)
	movq	%rdi, 176(%rsp)
	movdqa	%xmm3, 224(%rsp)
.LBB19_27:
	rep		bsfl	%r10d, %ebx
	addq	%rbp, %rbx
	andq	%rcx, %rbx
	negq	%rbx
	imulq	$56, %rbx, %rcx
	leaq	(%r8,%rcx), %rdi
	movq	-40(%r8,%rcx), %rax
	cmpq	$2, %rax
	movq	%r10, 72(%rsp)
	jbe	.LBB19_28
	movq	-48(%rdi), %rax
	movq	-56(%r8,%rcx), %rdi
	jmp	.LBB19_30
.LBB19_28:
	addq	$-48, %rdi
.LBB19_30:
	cmpq	%rsi, %rax
	jne	.LBB19_32
	movq	%r8, 56(%rsp)
	movq	%r9, %rsi
	movq	%r13, 48(%rsp)
	movq	%r9, %r13
	movq	%rdx, 80(%rsp)
	callq	*%r14
	movq	80(%rsp), %rdx
	movq	56(%rsp), %r8
	movq	%r13, %r9
	movq	48(%rsp), %r13
	movq	32(%rsp), %r10
	testl	%eax, %eax
	je	.LBB19_42
.LBB19_32:
	movq	72(%rsp), %rcx
	leal	-1(%rcx), %eax
	andw	%cx, %ax
	movl	%eax, %r10d
	movq	40(%rsp), %rcx
	movdqa	192(%rsp), %xmm1
	movq	64(%rsp), %rsi
	movq	176(%rsp), %rdi
	pcmpeqd	%xmm2, %xmm2
	movdqa	224(%rsp), %xmm3
	jne	.LBB19_27
.LBB19_33:
	pcmpeqb	%xmm2, %xmm3
	pmovmskb	%xmm3, %eax
	testl	%eax, %eax
	jne	.LBB19_54
	addq	%rdi, %rbp
	addq	$16, %rbp
	addq	$16, %rdi
	jmp	.LBB19_25
.LBB19_19:
	leaq	(,%r13,4), %rdx
	leaq	144(%rsp), %rsi
	xorl	%edi, %edi
	pcmpeqd	%xmm2, %xmm2
	movq	bcmp@GOTPCREL(%rip), %r14
.LBB19_20:
	andq	%rcx, %rbp
	movdqu	(%r8,%rbp), %xmm3
	movdqa	%xmm3, %xmm0
	pcmpeqb	%xmm1, %xmm0
	pmovmskb	%xmm0, %r10d
	testl	%r10d, %r10d
	je	.LBB19_39
	movq	%rcx, 40(%rsp)
	movdqa	%xmm1, 192(%rsp)
	movq	%rdi, 64(%rsp)
	movdqa	%xmm3, 176(%rsp)
.LBB19_22:
	rep		bsfl	%r10d, %ebx
	addq	%rbp, %rbx
	andq	%rcx, %rbx
	negq	%rbx
	imulq	$56, %rbx, %rdi
	leaq	(%r8,%rdi), %rcx
	movq	-40(%r8,%rdi), %rax
	cmpq	$3, %rax
	movq	%r10, 72(%rsp)
	jb	.LBB19_35
	movq	-56(%r8,%rdi), %rdi
	movq	-48(%rcx), %rax
	jmp	.LBB19_36
.LBB19_35:
	addq	$-48, %rcx
	movq	%rcx, %rdi
.LBB19_36:
	cmpq	%r13, %rax
	jne	.LBB19_38
	movq	%r8, 56(%rsp)
	movq	%r13, 48(%rsp)
	movq	%r9, %r13
	movq	%rdx, 80(%rsp)
	callq	*%r14
	leaq	144(%rsp), %rsi
	movq	80(%rsp), %rdx
	movq	56(%rsp), %r8
	movq	%r13, %r9
	movq	48(%rsp), %r13
	movq	32(%rsp), %r10
	testl	%eax, %eax
	je	.LBB19_42
.LBB19_38:
	movq	72(%rsp), %rcx
	leal	-1(%rcx), %eax
	andw	%cx, %ax
	movl	%eax, %r10d
	movq	40(%rsp), %rcx
	movdqa	192(%rsp), %xmm1
	movq	64(%rsp), %rdi
	pcmpeqd	%xmm2, %xmm2
	movdqa	176(%rsp), %xmm3
	jne	.LBB19_22
.LBB19_39:
	pcmpeqb	%xmm2, %xmm3
	pmovmskb	%xmm3, %eax
	testl	%eax, %eax
	jne	.LBB19_54
	addq	%rdi, %rbp
	addq	$16, %rbp
	addq	$16, %rdi
	jmp	.LBB19_20
.LBB19_42:
	imulq	$56, %rbx, %rax
	leaq	(%r8,%rax), %rbx
	movq	-8(%r8,%rax), %rax
	movq	%rax, %rdx
	cmpq	$3, %rax
	jb	.LBB19_44
	movq	-24(%rbx), %rdx
.LBB19_44:
	movq	(%r10), %rcx
	movq	16(%r10), %rsi
	subq	%rsi, %rcx
	cmpq	%rcx, %rdx
	ja	.LBB19_45
	cmpq	$3, %rax
	jb	.LBB19_48
.LBB19_47:
	movq	-24(%rbx), %rax
	movq	-16(%rbx), %rbx
	testq	%rax, %rax
	jne	.LBB19_50
	jmp	.LBB19_57
.LBB19_54:
	cmpq	$3, %r13
	jb	.LBB19_59
	movq	%r9, %rdi
	addq	$248, %rsp
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
.LBB19_45:
	.cfi_def_cfa_offset 304
	movl	$32, %ecx
	movq	%r10, %rdi
	movq	%r9, %r14
	callq	_ZN5alloc7raw_vec20RawVecInner$LT$A$GT$7reserve21do_reserve_and_handle17he8643a343e567234E
	movq	%r14, %r9
	movq	32(%rsp), %r10
	movq	-8(%rbx), %rax
	cmpq	$3, %rax
	jae	.LBB19_47
.LBB19_48:
	addq	$-24, %rbx
	testq	%rax, %rax
	je	.LBB19_57
.LBB19_50:
	xorl	%edx, %edx
	cmpq	$0, 88(%rsp)
	setne	%dl
	shll	$3, %edx
	cmpq	$5, %r15
	jb	.LBB19_51
	movq	8(%r12), %r15
	movq	16(%r12), %r12
	jmp	.LBB19_53
.LBB19_51:
	addq	$4, %r12
.LBB19_53:
	leaq	(%rbx,%rax,8), %rax
	movq	%rax, 40(%rsp)
	leaq	8(%rbx), %rax
	movq	168(%rsp), %rcx
	addq	%rdx, %rcx
	movq	%rcx, 48(%rsp)
	leaq	24(%rsp), %r11
	leaq	4(%rsp), %r9
	leaq	8(%rsp), %r8
	movq	88(%rsp), %rcx
	leaq	(,%rcx,8), %rcx
	subq	%rcx, %rdx
	movq	%rdx, 56(%rsp)
	jmp	.LBB19_98
	.p2align	4
.LBB19_97:
	movq	8(%r10), %rax
	movq	%r14, %rcx
	shlq	$5, %rcx
	movdqa	96(%rsp), %xmm0
	movdqa	112(%rsp), %xmm1
	movdqu	%xmm1, 16(%rax,%rcx)
	movdqu	%xmm0, (%rax,%rcx)
	incq	%r14
	movq	%r14, 16(%r10)
	xorl	%eax, %eax
	cmpq	40(%rsp), %rbx
	setne	%al
	leaq	(%rbx,%rax,8), %rax
	je	.LBB19_56
.LBB19_98:
	movq	(%rbx), %rdi
	cmpq	160(%rsp), %rdi
	jae	.LBB19_102
	movq	%rax, %rbx
	shlq	$5, %rdi
	movq	216(%rsp), %rax
	leaq	(%rax,%rdi), %r13
	movq	24(%rax,%rdi), %rax
	cmpq	$4, %rax
	jbe	.LBB19_100
	movq	8(%r13), %rax
	movq	16(%r13), %r13
	leaq	(,%rax,4), %r14
	addq	%r13, %r14
	movq	$0, 24(%rsp)
	movl	$0, (%rsp)
	movl	$4, %ebp
	cmpq	$5, %rax
	jb	.LBB19_61
	decq	%rax
	bsrq	%rax, %rcx
	notl	%ecx
	movq	$-1, %rsi
	shrq	%cl, %rsi
	incq	%rsi
	movq	%rsp, %rdi
	callq	_ZN8smallvec17SmallVec$LT$A$GT$8try_grow17hd7e1725870bd764fE
	movabsq	$-9223372036854775807, %rcx
	cmpq	%rcx, %rax
	jne	.LBB19_63
	movq	24(%rsp), %rsi
	cmpq	$5, %rsi
	jb	.LBB19_66
	movq	8(%rsp), %rax
	movq	16(%rsp), %rdx
	leaq	8(%rsp), %r8
	movq	%r8, %rcx
	movq	%rsi, %rbp
	movq	32(%rsp), %r10
	leaq	24(%rsp), %r11
	leaq	4(%rsp), %r9
	cmpq	%rbp, %rax
	jb	.LBB19_69
	jmp	.LBB19_74
	.p2align	4
.LBB19_100:
	leaq	4(,%rax,4), %r14
	addq	%r13, %r14
	addq	$4, %r13
	movq	$0, 24(%rsp)
	movl	$0, (%rsp)
	movl	$4, %ebp
.LBB19_61:
	xorl	%eax, %eax
	movq	%r9, %rdx
	movq	%r11, %rcx
	cmpq	%rbp, %rax
	jae	.LBB19_74
.LBB19_69:
	movq	%r14, %rdi
	subq	%r13, %rdi
	movq	%rdi, %r8
	shrq	$2, %r8
	movq	%rax, %rsi
	notq	%rsi
	addq	%rbp, %rsi
	cmpq	%rsi, %r8
	cmovbq	%r8, %rsi
	cmpq	$12, %rsi
	setae	%r8b
	testb	$3, %dil
	sete	%dil
	testb	%dil, %r8b
	je	.LBB19_70
	leaq	(%rdx,%rax,4), %rdi
	subq	%r13, %rdi
	cmpq	$32, %rdi
	jb	.LBB19_70
	incq	%rsi
	leaq	(,%rax,4), %r8
	movl	%esi, %edi
	andl	$7, %edi
	movl	$8, %r9d
	cmoveq	%r9, %rdi
	subq	%rdi, %rsi
	leaq	(,%rsi,4), %rdi
	addq	%rdx, %r8
	addq	$16, %r8
	xorl	%r9d, %r9d
	.p2align	4
.LBB19_80:
	movdqu	(%r13,%r9,4), %xmm0
	movdqu	16(%r13,%r9,4), %xmm1
	movdqu	%xmm0, -16(%r8,%r9,4)
	movdqu	%xmm1, (%r8,%r9,4)
	addq	$8, %r9
	cmpq	%r9, %rsi
	jne	.LBB19_80
	addq	%rsi, %rax
	addq	%rdi, %r13
	leaq	4(%rsp), %r9
.LBB19_70:
	leaq	8(%rsp), %r8
	.p2align	4
.LBB19_71:
	cmpq	%r14, %r13
	je	.LBB19_86
	movl	(%r13), %esi
	addq	$4, %r13
	movl	%esi, (%rdx,%rax,4)
	incq	%rax
	cmpq	%rax, %rbp
	jne	.LBB19_71
	movq	%rbp, %rax
	movq	%rax, (%rcx)
	cmpq	%r14, %r13
	jne	.LBB19_76
	jmp	.LBB19_87
	.p2align	4
.LBB19_86:
	movq	%rax, (%rcx)
.LBB19_87:
	movdqu	(%rsp), %xmm0
	movdqu	16(%rsp), %xmm1
	movdqa	%xmm1, 112(%rsp)
	movdqa	%xmm0, 96(%rsp)
	movq	48(%rsp), %rax
	movq	56(%rsp), %r14
	movq	168(%rsp), %r13
	cmpq	$0, 88(%rsp)
	je	.LBB19_95
	.p2align	4
.LBB19_88:
	movq	(%r13), %rdi
	cmpq	%r15, %rdi
	jae	.LBB19_103
	movq	%rax, %r13
	movq	120(%rsp), %rsi
	cmpq	$5, %rsi
	jb	.LBB19_90
	movq	104(%rsp), %rax
	movq	112(%rsp), %rcx
	leaq	104(%rsp), %rdx
	movl	(%r12,%rdi,4), %ebp
	cmpq	%rsi, %rax
	je	.LBB19_93
.LBB19_94:
	movl	%ebp, (%rcx,%rax,4)
	incq	(%rdx)
	addq	$8, %r14
	leaq	8(%r13), %rax
	cmpq	$8, %r14
	jne	.LBB19_88
	jmp	.LBB19_95
	.p2align	4
.LBB19_90:
	movq	%rsi, %rax
	leaq	100(%rsp), %rcx
	leaq	120(%rsp), %rdx
	movl	$4, %esi
	movl	(%r12,%rdi,4), %ebp
	cmpq	%rsi, %rax
	jne	.LBB19_94
.LBB19_93:
	leaq	96(%rsp), %rdi
	callq	_ZN8smallvec17SmallVec$LT$A$GT$21reserve_one_unchecked17hf5e5c6f35dcd7048E
	leaq	8(%rsp), %r8
	leaq	4(%rsp), %r9
	leaq	24(%rsp), %r11
	movq	32(%rsp), %r10
	movq	104(%rsp), %rax
	movq	112(%rsp), %rcx
	leaq	104(%rsp), %rdx
	jmp	.LBB19_94
	.p2align	4
.LBB19_95:
	movq	16(%r10), %r14
	cmpq	(%r10), %r14
	jne	.LBB19_97
	movq	%r10, %rdi
	callq	_ZN5alloc7raw_vec19RawVec$LT$T$C$A$GT$8grow_one17h6541dc952f3628c7E
	leaq	8(%rsp), %r8
	leaq	4(%rsp), %r9
	leaq	24(%rsp), %r11
	movq	32(%rsp), %r10
	jmp	.LBB19_97
.LBB19_66:
	movq	%rsi, %rax
	leaq	4(%rsp), %r9
	movq	%r9, %rdx
	leaq	24(%rsp), %r11
	movq	%r11, %rcx
	movq	32(%rsp), %r10
	leaq	8(%rsp), %r8
	cmpq	%rbp, %rax
	jb	.LBB19_69
	.p2align	4
.LBB19_74:
	movq	%rax, (%rcx)
	cmpq	%r14, %r13
	je	.LBB19_87
	.p2align	4
.LBB19_76:
	movq	24(%rsp), %rsi
	cmpq	$5, %rsi
	jb	.LBB19_77
	movq	8(%rsp), %rax
	movq	16(%rsp), %rcx
	movq	%r8, %rdx
	movl	(%r13), %ebp
	cmpq	%rsi, %rax
	je	.LBB19_84
.LBB19_85:
	addq	$4, %r13
	movl	%ebp, (%rcx,%rax,4)
	incq	(%rdx)
	cmpq	%r14, %r13
	jne	.LBB19_76
	jmp	.LBB19_87
	.p2align	4
.LBB19_77:
	movq	%rsi, %rax
	movq	%r9, %rcx
	movq	%r11, %rdx
	movl	$4, %esi
	movl	(%r13), %ebp
	cmpq	%rsi, %rax
	jne	.LBB19_85
.LBB19_84:
	movq	%rsp, %rdi
	callq	_ZN8smallvec17SmallVec$LT$A$GT$21reserve_one_unchecked17hf5e5c6f35dcd7048E
	leaq	8(%rsp), %r8
	leaq	4(%rsp), %r9
	leaq	24(%rsp), %r11
	movq	32(%rsp), %r10
	movq	8(%rsp), %rax
	movq	16(%rsp), %rcx
	movq	%r8, %rdx
	jmp	.LBB19_85
.LBB19_56:
	movq	136(%rsp), %r9
	movq	152(%rsp), %r13
.LBB19_57:
	cmpq	$3, %r13
	jb	.LBB19_59
	movq	%r9, %rdi
	callq	*free@GOTPCREL(%rip)
.LBB19_59:
	addq	$248, %rsp
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
.LBB19_63:
	.cfi_def_cfa_offset 304
	testq	%rax, %rax
	je	.LBB19_64
	movq	%rax, %rdi
	movq	%rdx, %rsi
	callq	_ZN5alloc5alloc18handle_alloc_error17h29c279d8237d34e5E
.LBB19_103:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.30(%rip), %rdx
	movq	%r15, %rsi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB19_102:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.28(%rip), %rdx
	movq	160(%rsp), %rsi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB19_41:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.27(%rip), %rdx
	movq	%r14, %rsi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB19_64:
	leaq	.Lanon.38cf5e84a9682489615e8b34a43bde4a.10(%rip), %rdi
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.51(%rip), %rdx
	movl	$17, %esi
	callq	_ZN4core9panicking5panic17h4a11c031239f36a8E
.Lfunc_end19:
	.size	audit_probe_emit, .Lfunc_end19-audit_probe_emit
