_ZN10sparq_core4dict4Dict8find_iri17hd5483d417ec6bb45E:
.Lfunc_begin0:
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
	subq	$168, %rsp
	.cfi_def_cfa_offset 224
	.cfi_offset %rbx, -56
	.cfi_offset %r12, -48
	.cfi_offset %r13, -40
	.cfi_offset %r14, -32
	.cfi_offset %r15, -24
	.cfi_offset %rbp, -16
	movq	%rcx, %r9
	movq	%rdx, %r10
	movq	%rsi, %r13
	movq	%rdi, %r14
	movq	224(%rdi), %rdi
	testq	%rdi, %rdi
	je	.LBB0_4
	addq	$16, %rdi
	movq	%r13, %rsi
	movq	%r10, %rdx
	movq	%r9, %rcx
	movq	%r9, %rbx
	movq	%r10, %r15
	callq	_ZN10sparq_core4dict4Dict8find_iri17hd5483d417ec6bb45E
	movq	%r15, %r10
	movq	%rbx, %r9
	testb	$1, %al
	je	.LBB0_4
	movl	%edx, %ebp
.LBB0_3:
	movl	%ebp, %edx
	addq	$168, %rsp
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
.LBB0_4:
	.cfi_def_cfa_offset 224
	movq	%r13, %rax
	shrq	$57, %rax
	movq	192(%r14), %r8
	movq	184(%r14), %r11
	movd	%eax, %xmm0
	punpcklbw	%xmm0, %xmm0
	pshuflw	$0, %xmm0, %xmm0
	pshufd	$0, %xmm0, %xmm1
	movq	216(%r14), %rbx
	movq	64(%r14), %rax
	movq	%rax, 56(%rsp)
	movq	56(%r14), %rax
	movq	%rax, 64(%rsp)
	movq	8(%r14), %rax
	movq	%rax, 8(%rsp)
	movq	16(%r14), %rax
	movq	%rax, (%rsp)
	movq	72(%r14), %rax
	movq	%rax, 136(%rsp)
	movq	112(%r14), %rax
	movq	%rax, 48(%rsp)
	movq	104(%r14), %rax
	movq	%rax, 128(%rsp)
	movq	88(%r14), %r15
	movq	80(%r14), %rax
	movq	%rax, 120(%rsp)
	movq	$0, 40(%rsp)
.LBB0_5:
	andq	%r8, %r13
	movdqu	(%r11,%r13), %xmm0
	movdqa	%xmm0, 144(%rsp)
	pcmpeqb	%xmm1, %xmm0
	pmovmskb	%xmm0, %r14d
	testl	%r14d, %r14d
	je	.LBB0_88
	movq	%r13, 104(%rsp)
	movq	%r8, 96(%rsp)
	movq	%r11, 88(%rsp)
	movq	%rbx, 80(%rsp)
	movq	%r15, 72(%rsp)
.LBB0_7:
	rep		bsfl	%r14d, %eax
	addq	%r13, %rax
	andq	%r8, %rax
	shlq	$2, %rax
	movq	%r11, %rcx
	subq	%rax, %rcx
	movl	-4(%rcx), %ebp
	leal	-1(%rbp), %edi
	movq	%rdi, %rax
	subq	%rbx, %rax
	movq	%r14, 112(%rsp)
	jae	.LBB0_76
	movq	136(%rsp), %rax
	negq	%rax
	jo	.LBB0_92
	cmpq	%rdi, 48(%rsp)
	jbe	.LBB0_93
	movq	128(%rsp), %rax
	movl	(%rax,%rdi,4), %r12d
	cmpq	%r12, %r15
	jb	.LBB0_72
	movq	%r15, %rsi
	subq	%r12, %rsi
	je	.LBB0_94
	addq	120(%rsp), %r12
	movzbl	(%r12), %eax
	testl	%eax, %eax
	je	.LBB0_22
	cmpl	$1, %eax
	je	.LBB0_39
	cmpl	$3, %eax
	jne	.LBB0_15
	cmpq	$1, %rsi
	je	.LBB0_98
	cmpq	$2, %rsi
	jbe	.LBB0_99
	cmpq	$3, %rsi
	je	.LBB0_100
	cmpq	$4, %rsi
	jbe	.LBB0_95
	cmpq	$5, %rsi
	je	.LBB0_101
	cmpq	$6, %rsi
	jbe	.LBB0_102
	cmpq	$7, %rsi
	je	.LBB0_103
	cmpq	$8, %rsi
	jbe	.LBB0_96
	cmpq	$9, %rsi
	je	.LBB0_104
	cmpq	$10, %rsi
	jbe	.LBB0_105
	cmpq	$11, %rsi
	je	.LBB0_106
	cmpq	$12, %rsi
	jbe	.LBB0_67
	jmp	.LBB0_87
	.p2align	4
.LBB0_76:
	cmpq	56(%rsp), %rax
	jae	.LBB0_91
	leaq	(%rax,%rax,4), %rax
	movq	64(%rsp), %rcx
	cmpl	$0, (%rcx,%rax,8)
	jne	.LBB0_87
	movq	64(%rsp), %rcx
	leaq	(%rcx,%rax,8), %r13
	movl	4(%r13), %edi
	cmpq	%rdi, (%rsp)
	jbe	.LBB0_107
	shlq	$4, %rdi
	movq	8(%rsp), %rax
	movq	8(%rax,%rdi), %r14
	cmpq	%r14, %r9
	jb	.LBB0_87
	movq	16(%r13), %r12
	leaq	(%r12,%r14), %rax
	cmpq	%rax, %r9
	jne	.LBB0_87
	addq	8(%rsp), %rdi
	movq	(%rdi), %rdi
	movq	%r10, %rsi
	movq	%r14, %rdx
	movq	%r9, %rbx
	movq	%r10, %r15
	movdqa	%xmm1, 16(%rsp)
	callq	*bcmp@GOTPCREL(%rip)
	movdqa	16(%rsp), %xmm1
	movq	%r15, %r10
	movq	%rbx, %r9
	testl	%eax, %eax
	jne	.LBB0_87
	testq	%r14, %r14
	sete	%cl
	movq	%r9, %rax
	subq	%r14, %rax
	setbe	%dl
	orb	%cl, %dl
	jne	.LBB0_84
	cmpb	$-64, (%r10,%r14)
	jl	.LBB0_90
.LBB0_84:
	cmpq	%r12, %rax
	jne	.LBB0_87
	movq	8(%r13), %rsi
	addq	%r10, %r14
	movq	%r14, %rdi
	movq	%r12, %rdx
	callq	*bcmp@GOTPCREL(%rip)
	movdqa	16(%rsp), %xmm1
	movq	%r15, %r10
	movq	%rbx, %r9
	movl	%eax, %ecx
	movl	$1, %eax
	testl	%ecx, %ecx
	je	.LBB0_3
	jmp	.LBB0_87
.LBB0_39:
	cmpq	$1, %rsi
	je	.LBB0_98
	cmpq	$2, %rsi
	jbe	.LBB0_99
	cmpq	$3, %rsi
	je	.LBB0_100
	cmpq	$4, %rsi
	jbe	.LBB0_95
	movzbl	1(%r12), %eax
	movzbl	2(%r12), %ecx
	movzbl	3(%r12), %edx
	movzbl	4(%r12), %edi
	shll	$24, %edi
	shll	$16, %edx
	shll	$8, %ecx
	orq	%rax, %rcx
	orq	%rdx, %rcx
	leaq	(%rcx,%rdi), %rax
	addq	%rcx, %rdi
	addq	$5, %rdi
	cmpq	%rsi, %rdi
	ja	.LBB0_21
	jae	.LBB0_69
	leaq	6(%rax), %rdi
	cmpq	%rsi, %rdi
	jae	.LBB0_70
	leaq	7(%rax), %rdi
	cmpq	%rsi, %rdi
	jae	.LBB0_71
	leaq	8(%rax), %rdi
	cmpq	%rsi, %rdi
	jae	.LBB0_97
	leaq	9(%rax), %rdi
	cmpq	%rsi, %rdi
	jae	.LBB0_68
	cmpb	$1, (%r12,%rdi)
	jne	.LBB0_87
	leaq	10(%rax), %rdi
	cmpq	%rsi, %rdi
	jae	.LBB0_69
	leaq	11(%rax), %rdi
	cmpq	%rsi, %rdi
	jae	.LBB0_70
	leaq	12(%rax), %rdi
	cmpq	%rsi, %rdi
	jae	.LBB0_71
	leaq	13(%rax), %rdi
	cmpq	%rsi, %rdi
	jae	.LBB0_97
	movzbl	10(%r12,%rax), %ecx
	movzbl	11(%r12,%rax), %edx
	movzbl	12(%r12,%rax), %edi
	movzbl	13(%r12,%rax), %r8d
	shll	$24, %r8d
	shll	$16, %edi
	shll	$8, %edx
	orq	%rcx, %rdx
	orq	%rdi, %rdx
	orq	%r8, %rdx
	leaq	(%rax,%rdx), %rdi
	addq	$14, %rdi
	jmp	.LBB0_20
.LBB0_22:
	cmpq	$1, %rsi
	je	.LBB0_98
	cmpq	$2, %rsi
	jbe	.LBB0_99
	cmpq	$3, %rsi
	je	.LBB0_100
	cmpq	$4, %rsi
	jbe	.LBB0_95
	cmpq	$5, %rsi
	je	.LBB0_101
	cmpq	$6, %rsi
	jbe	.LBB0_102
	cmpq	$7, %rsi
	je	.LBB0_103
	cmpq	$8, %rsi
	jbe	.LBB0_96
	movzbl	5(%r12), %ecx
	movzbl	6(%r12), %r13d
	movzbl	7(%r12), %edx
	movzbl	8(%r12), %eax
	shll	$24, %eax
	shll	$16, %edx
	shll	$8, %r13d
	orq	%rcx, %r13
	orq	%rdx, %r13
	leaq	(%rax,%r13), %rdi
	addq	$9, %rdi
	cmpq	%rsi, %rdi
	ja	.LBB0_21
	movzbl	1(%r12), %ecx
	movzbl	2(%r12), %edi
	movzbl	3(%r12), %edx
	movzbl	4(%r12), %esi
	shll	$24, %esi
	shll	$16, %edx
	shll	$8, %edi
	orq	%rcx, %rdi
	orq	%rdx, %rdi
	orq	%rsi, %rdi
	cmpq	%rdi, (%rsp)
	jbe	.LBB0_73
	shlq	$4, %rdi
	movq	8(%rsp), %rcx
	movq	8(%rcx,%rdi), %r14
	cmpq	%r14, %r9
	jb	.LBB0_87
	orq	%rax, %r13
	leaq	(%r14,%r13), %rax
	cmpq	%rax, %r9
	jne	.LBB0_87
	addq	8(%rsp), %rdi
	movq	(%rdi), %rdi
	movq	%r10, %rsi
	movq	%r14, %rdx
	movq	%r9, 32(%rsp)
	movq	%r10, %rbx
	movdqa	%xmm1, 16(%rsp)
	callq	*bcmp@GOTPCREL(%rip)
	movdqa	16(%rsp), %xmm1
	movq	%rbx, %r10
	movq	32(%rsp), %r9
	testl	%eax, %eax
	jne	.LBB0_87
	testq	%r14, %r14
	sete	%cl
	movq	%r9, %rax
	subq	%r14, %rax
	setbe	%dl
	orb	%cl, %dl
	jne	.LBB0_37
	cmpb	$-64, (%r10,%r14)
	jl	.LBB0_74
.LBB0_37:
	cmpq	%r13, %rax
	jne	.LBB0_87
	addq	$9, %r12
	addq	%r10, %r14
	movq	%r14, %rdi
	movq	%r12, %rsi
	movq	%r13, %rdx
	callq	*bcmp@GOTPCREL(%rip)
	movdqa	16(%rsp), %xmm1
	movq	%rbx, %r10
	movq	32(%rsp), %r9
	movl	%eax, %ecx
	movl	$1, %eax
	testl	%ecx, %ecx
	je	.LBB0_3
	jmp	.LBB0_87
.LBB0_15:
	cmpq	$1, %rsi
	je	.LBB0_98
	cmpq	$2, %rsi
	jbe	.LBB0_99
	cmpq	$3, %rsi
	je	.LBB0_100
	cmpq	$4, %rsi
	jbe	.LBB0_95
	movzbl	1(%r12), %eax
	movzbl	2(%r12), %ecx
	movzbl	3(%r12), %edx
	movzbl	4(%r12), %edi
	shll	$24, %edi
	shll	$16, %edx
	shll	$8, %ecx
	orq	%rax, %rcx
	orq	%rdx, %rcx
	addq	%rcx, %rdi
	addq	$5, %rdi
.LBB0_20:
	cmpq	%rsi, %rdi
	ja	.LBB0_21
.LBB0_87:
	movq	112(%rsp), %rcx
	leal	-1(%rcx), %eax
	andw	%cx, %ax
	movl	%eax, %r14d
	movq	104(%rsp), %r13
	movq	96(%rsp), %r8
	movq	88(%rsp), %r11
	movq	80(%rsp), %rbx
	movq	72(%rsp), %r15
	jne	.LBB0_7
	.p2align	4
.LBB0_88:
	movdqa	144(%rsp), %xmm0
	pcmpeqb	.LCPI0_0(%rip), %xmm0
	pmovmskb	%xmm0, %ecx
	xorl	%eax, %eax
	testl	%ecx, %ecx
	jne	.LBB0_3
	movq	40(%rsp), %rax
	addq	%rax, %r13
	addq	$16, %r13
	addq	$16, %rax
	movq	%rax, 40(%rsp)
	jmp	.LBB0_5
.LBB0_67:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.162(%rip), %rdx
	movl	$12, %edi
	movl	$12, %esi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_92:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.12(%rip), %rdi
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.13(%rip), %rdx
	movl	$42, %esi
	callq	_ZN4core6option13expect_failed17h1fcc4e32848a6083E
.LBB0_72:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.15(%rip), %rdx
	movq	%r12, %rdi
	movq	%r15, %rsi
	callq	_ZN4core5slice5index26slice_start_index_len_fail17h0596e605fb4610d0E
.LBB0_21:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.164(%rip), %rdx
	callq	_ZN4core5slice5index24slice_end_index_len_fail17hb6890d29d4255062E
.LBB0_91:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.11(%rip), %rdx
	movq	%rax, %rdi
	movq	56(%rsp), %rsi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_99:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.160(%rip), %rdx
	movl	$2, %edi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_95:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.162(%rip), %rdx
	movl	$4, %edi
	movl	$4, %esi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_100:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.161(%rip), %rdx
	movl	$3, %edi
	movl	$3, %esi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_98:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.159(%rip), %rdx
	movl	$1, %edi
	movl	$1, %esi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_94:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.163(%rip), %rdx
	xorl	%edi, %edi
	xorl	%esi, %esi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_107:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.3(%rip), %rdx
	movq	(%rsp), %rsi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_93:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.14(%rip), %rdx
	movq	48(%rsp), %rsi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_69:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.159(%rip), %rdx
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_70:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.160(%rip), %rdx
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_71:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.161(%rip), %rdx
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_97:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.162(%rip), %rdx
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_102:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.160(%rip), %rdx
	movl	$6, %edi
	movl	$6, %esi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_96:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.162(%rip), %rdx
	movl	$8, %edi
	movl	$8, %esi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_103:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.161(%rip), %rdx
	movl	$7, %edi
	movl	$7, %esi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_101:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.159(%rip), %rdx
	movl	$5, %edi
	movl	$5, %esi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_90:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.4(%rip), %r8
.LBB0_75:
	movq	%r10, %rdi
	movq	%r9, %rsi
	movq	%r14, %rdx
	movq	%r9, %rcx
	callq	_ZN4core3str16slice_error_fail17h9f974238edffa500E
.LBB0_105:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.160(%rip), %rdx
	movl	$10, %edi
	movl	$10, %esi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_106:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.161(%rip), %rdx
	movl	$11, %edi
	movl	$11, %esi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_104:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.159(%rip), %rdx
	movl	$9, %edi
	movl	$9, %esi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_68:
	leaq	.Lanon.f8b7d957e2fe18c6f7aeff650025346a.165(%rip), %rdx
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_73:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.8(%rip), %rdx
	movq	(%rsp), %rsi
	callq	_ZN4core9panicking18panic_bounds_check17hda0827d94e974e71E
.LBB0_74:
	leaq	.Lanon.77819ad2770acbf4c337de940308a095.9(%rip), %r8
	jmp	.LBB0_75
.Lfunc_end0:
	.size	_ZN10sparq_core4dict4Dict8find_iri17hd5483d417ec6bb45E, .Lfunc_end0-_ZN10sparq_core4dict4Dict8find_iri17hd5483d417ec6bb45E
