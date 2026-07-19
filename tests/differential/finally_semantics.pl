#!/usr/bin/perl
# Differential tester for nushell try/catch/finally control flow.
#
#   perl tests/differential/finally_semantics.pl [path-to-nu]   (default ./target/debug/nu)
#
# It enumerates try/catch/finally programs (every exit kind pending: value, return, throw,
# break, continue, exit; nested trys in the body and finally; catch inside vs outside; with and
# without an enclosing loop) and checks each against a reference model that encodes the
# abrupt-completion rules of JLS 14.20.2 / Python. Every leaf prints a unique marker, so each
# program is compared on both its ordered side-effect trace and its final outcome
# (returned value / thrown error / exit code). Prints "ran N programs, M mismatches" and details
# for each mismatch; exits non-zero if any. The large loop set is strided for runtime; drop the
# strides in the `@combos` section for an exhaustive sweep.
use strict; use warnings;
my $NU = $ARGV[0] // "./target/debug/nu";

# AST node = arrayref. Leaves: [K, id] with K in V R B C T. Composites:
#   [seq, [nodes...]] ; [try, body, catch_or_undef, fin_or_undef] ; [loop, body]

# ---- reference model: ev(node) -> ([markers], [kind, payload]) --------------
sub is_abrupt { $_[0][0] ne 'normal' }
sub ev {
  my ($n) = @_; my $k = $n->[0];
  return ([$n->[1]], ['normal',  $n->[1]]) if $k eq 'V';
  return ([$n->[1]], ['return',  $n->[1]]) if $k eq 'R';
  return ([$n->[1]], ['break',   undef])   if $k eq 'B';
  return ([$n->[1]], ['continue',undef])   if $k eq 'C';
  return ([$n->[1]], ['throw',   $n->[1]]) if $k eq 'T';
  return ([$n->[1]], ['exit',    7])       if $k eq 'X';
  if ($k eq 'seq') {
    my @tr; my $out = ['normal','empty'];
    for my $c (@{$n->[1]}) { my ($t,$o)=ev($c); push @tr,@$t; $out=$o; return (\@tr,$o) if is_abrupt($o); }
    return (\@tr, $out);
  }
  if ($k eq 'try') {
    my (undef,$body,$catch,$fin) = @$n;
    my ($tb,$ob)=ev($body); my @tr=@$tb; my $res=$ob;
    if ($ob->[0] eq 'throw' && defined $catch) { my ($tc,$oc)=ev($catch); push @tr,@$tc; $res=$oc; }
    if (defined $fin) { my ($tf,$of)=ev($fin); push @tr,@$tf; $res=$of if is_abrupt($of); }
    return (\@tr, $res);
  }
  if ($k eq 'loop') {
    my @tr;
    for (1,2) { my ($tb,$ob)=ev($n->[1]); push @tr,@$tb;
      return (\@tr,['normal','empty']) if $ob->[0] eq 'break';
      next if $ob->[0] eq 'continue';
      return (\@tr,$ob) if is_abrupt($ob); }
    return (\@tr, ['normal','empty']);
  }
  die "bad node $k";
}
sub top_outcome { my ($o)=@_; my ($k,$p)=@$o;
  return ['VALUE',$p] if $k eq 'normal' || $k eq 'return';
  return ['THROW',$p] if $k eq 'throw';
  return [uc($k),$p]; }

# ---- nushell emitter --------------------------------------------------------
sub emit {
  my ($n)=@_; my $k=$n->[0]; my $id=$n->[1]//'';
  return "print \"$id\"\n\"$id\"" if $k eq 'V';
  return "print \"$id\"\nreturn \"$id\""       if $k eq 'R';
  return "print \"$id\"\nbreak"                if $k eq 'B';
  return "print \"$id\"\ncontinue"             if $k eq 'C';
  return "print \"$id\"\nerror make { msg: \"$id\" }" if $k eq 'T';
  return "print \"$id\"\nexit 7" if $k eq 'X';
  if ($k eq 'seq') { return join("\n", map { emit($_) } @{$n->[1]}); }
  if ($k eq 'try') {
    my (undef,$b,$c,$f)=@$n;
    my $s = "try {\n".emit($b)."\n}";
    $s .= " catch {|err|\n".emit($c)."\n}" if defined $c;
    $s .= " finally {\n".emit($f)."\n}" if defined $f;
    return $s;
  }
  if ($k eq 'loop') { return "for _gv in [1 2] {\n".emit($n->[1])."\n}"; }
  die "bad node $k";
}
sub program_source {
  my ($n)=@_;
  return "def prog [] {\n".emit($n)."\n}\n".
         "let out = (try { let v = (prog); \$\"VALUE:(\$v | to nuon)\" } catch {|e| \$\"THROW:(\$e.msg)\" })\n".
         "print \$\"OUTCOME:(\$out)\"\n";
}

# ---- run nu -----------------------------------------------------------------
my $TMP = "/tmp/nu-finally-diff-$$.nu";
sub run_nu {
  my ($n)=@_; my $src=program_source($n);
  open(my $fh, '>', $TMP) or die "open $TMP: $!"; print $fh $src; close $fh;
  my $out = `$NU --no-config-file $TMP 2>/dev/null`;
  my $rc = $? >> 8;
  my @markers; my $outcome;
  for my $line (split /\n/, $out) {
    $line =~ s/^\s+|\s+$//g;
    if ($line =~ /^OUTCOME:VALUE:(.*)$/) { my $v=$1; $v=~s/^"|"$//g; $outcome=['VALUE', ($v eq '' || $v eq 'null')?'empty':$v]; }
    elsif ($line =~ /^OUTCOME:THROW:(.*)$/) { $outcome=['THROW',$1]; }
    elsif ($line ne '') { push @markers,$line; }
  }
  $outcome //= ['EXIT',$rc];
  return (\@markers,$outcome,$src);
}

sub eq_list { my ($a,$b)=@_; return 0 unless @$a==@$b; for (0..$#$a){return 0 if $a->[$_] ne $b->[$_];} return 1; }
sub eq_out  { my ($a,$b)=@_; return 0 if $a->[0] ne $b->[0]; my $x=$a->[1]//''; my $y=$b->[1]//''; return $x eq $y; }

# ---- generator (bounded, focused on nested-try-in-finally interactions) ------
my $ctr=0; sub fresh { return "m".($ctr++); }
sub leaf_kinds { my ($loop)=@_; return ('V','R','T','X', ($loop?('B','C'):())); }
sub leaf { my ($k)=@_; return [$k, fresh()]; }
sub leaves { my ($loop)=@_; return map { leaf($_) } leaf_kinds($loop); }
# nested unit: try { bl } [catch cl] [finally fl] -- one level, leaf parts
sub units {
  my ($loop)=@_; my @o;
  for my $bk (leaf_kinds($loop)) {
    for my $ck (undef, leaf_kinds($loop)) {
      for my $fk (undef, leaf_kinds($loop)) {
        next if !defined($ck) && !defined($fk);
        push @o, ['try', leaf($bk),
                  (defined $ck ? leaf($ck) : undef),
                  (defined $fk ? leaf($fk) : undef)];
      }
    }
  }
  return @o;
}
# top: [loop] try { body-leaf } [catch] finally|catch { slot } where a slot may be a nested unit
sub tops {
  my ($loop)=@_; my @o;
  my @fins   = (leaves($loop), units($loop));               # finally can nest a try
  my @bodies = (leaves($loop), units($loop));               # body can nest a try
  my @cbk    = (undef, leaf_kinds($loop));                  # catch body kinds (or none)
  # body is a nested unit or leaf; keep the outer catch a leaf and the finally rich
  for my $b (@bodies) {
    for my $ck (@cbk) {
      my $c = defined $ck ? leaf($ck) : undef;
      for my $f (@fins, undef) {
        next if !defined($c) && !defined($f);
        # bound: skip body-unit + finally-unit combos to keep the count sane
        next if $b->[0] eq 'try' && defined $f && $f->[0] eq 'try';
        push @o, ['try', $b, $c, $f];
      }
    }
  }
  return @o;
}

# ---- main -------------------------------------------------------------------
my @combos;
my @nl = tops(0);
for (my $i=0; $i<@nl; $i+=2) { push @combos, $nl[$i]; }   # stride no-loop set
my @lp = map { ['loop',$_] } tops(1);
for (my $i=0; $i<@lp; $i+=16) { push @combos, $lp[$i]; }  # stride loop set
printf STDERR "generated %d programs\n", scalar(@combos);
my $n=0; my @fails;
for my $node (@combos) {
  $n++;
  my ($et,$eo)=ev($node); my $etop=top_outcome($eo);
  my ($at,$ao,$src)=run_nu($node);
  unless (eq_list($et,$at) && eq_out($etop,$ao)) {
    push @fails, [$node,$et,$at,$etop,$ao,$src];
  }
}
printf "ran %d programs, %d mismatches\n", $n, scalar(@fails);
my $shown=0;
for my $f (@fails) {
  last if $shown++ >= 20;
  my ($node,$et,$at,$eo,$ao,$src)=@$f;
  print "\n--- MISMATCH ---\n";
  (my $flat=$src)=~s/\n/ /g; print "prog: ", substr($flat,0,320), "\n";
  print "expected trace: [@$et] outcome: [@$eo]\n";
  print "actual   trace: [@$at] outcome: [@$ao]\n";
}
exit(scalar(@fails) ? 1 : 0);
